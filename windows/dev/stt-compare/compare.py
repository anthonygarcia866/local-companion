"""Voice + Actions 1.5a: the speech-to-text comparison (docs/voice-actions-plan.md §3).

Dev-only; never ships. Makes the clips in clips.json with Windows' offline
voices (System.Speech, no network), runs whisper.cpp's whisper-cli on each
with each model, and scores:
  - WER against the reference (lowercase, punctuation and $ , % removed),
  - whether a command clip's transcript starts with its command word(s)
    (Glim's no-Claude "safe to type" rule, §4.1),
  - the names/numbers/domain words that came through,
  - processing time (whisper's own total minus model load: what Glim would
    pay with the model kept loaded) and wall time per clip.

Usage (from windows/dev/stt-compare):
  python -I compare.py [--prompt "<vocabulary hint>"] <work dir> <whisper-cli.exe> <model.bin> [<model.bin> ...]
With --prompt, whisper gets the user's names and words as its initial
prompt (the local counterpart of the cloud services' keyterms).
The work dir gets clips/ and results.json; nothing is written in the repo.
"""

import json
import os
import random
import re
import subprocess
import sys
import time
import wave

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
FILLER = ("hey glim", "um", "uh", "okay", "ok")


def norm(text: str) -> str:
    t = text.lower().replace("-", " ").replace("’", "'")
    t = re.sub(r"(\d),(\d)", r"\1\2", t)
    t = re.sub(r"[^a-z0-9' ]+", " ", t)
    return re.sub(r"\s+", " ", t).strip()


ORDINALS = {"first": "1", "second": "2", "third": "3", "fourth": "4", "fifth": "5", "sixth": "6", "seventh": "7", "eighth": "8", "ninth": "9", "tenth": "10"}


def lenient(text: str) -> str:
    """norm, plus formatting that isn't a mishearing: "24th" = "24",
    "first" = "1", "9 a.m." = "9am" = "9 am"."""
    t = norm(text)
    t = re.sub(r"\b(\d+)(st|nd|rd|th)\b", r"\1", t)
    t = " ".join(ORDINALS.get(w, w) for w in t.split())
    t = re.sub(r"\b(\d+) ?a ?m\b", r"\1 am", t)
    t = re.sub(r"\b(\d+) ?p ?m\b", r"\1 pm", t)
    return t


def collapse_digits(t: str) -> str:
    """'555 0198' -> '5550198': digit groups the model split or joined."""
    return re.sub(r"(?<=\d) (?=\d)", "", t)


def wer(ref: str, hyp: str) -> tuple[int, int]:
    r, h = ref.split(), hyp.split()
    d = list(range(len(h) + 1))
    for i in range(1, len(r) + 1):
        prev, d[0] = d[0], i
        for j in range(1, len(h) + 1):
            cur = min(d[j] + 1, d[j - 1] + 1, prev + (r[i - 1] != h[j - 1]))
            prev, d[j] = d[j], cur
    return d[len(h)], len(r)


def starts_with_command(hyp: str, command: str) -> bool:
    t = hyp
    changed = True
    while changed:
        changed = False
        for f in FILLER:
            if t.startswith(f + " "):
                t = t[len(f) + 1:]
                changed = True
    return t == command or t.startswith(command + " ")


def term_hit(hyp: str, term: str) -> bool:
    hyps = {f" {hyp} ", f" {collapse_digits(hyp)} "}
    return any(f" {norm(alt)} " in h or f" {collapse_digits(norm(alt))} " in h for alt in term.split("|") for h in hyps)


def make_clips(clips, out_dir):
    os.makedirs(out_dir, exist_ok=True)
    ps = []
    for c in clips:
        path = os.path.join(out_dir, c["id"] + ".wav")
        ssml = (
            '<speak version="1.0" xmlns="http://www.w3.org/2001/10/synthesis" xml:lang="en-US">'
            f'<voice name="Microsoft {c["voice"]} Desktop">{c["say"].replace("&", "and")}</voice></speak>'
        )
        ps.append(
            "$s = New-Object System.Speech.Synthesis.SpeechSynthesizer; "
            f"$s.Rate = {int(c.get('rate', 0))}; "
            "$f = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono); "
            f"$s.SetOutputToWaveFile('{path}', $f); $s.SpeakSsml(@'\n{ssml}\n'@); $s.Dispose();"
        )
    script = "Add-Type -AssemblyName System.Speech\n" + "\n".join(ps)
    subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", script], check=True)
    for c in clips:
        if c.get("noise_snr_db") is not None:
            add_noise(os.path.join(out_dir, c["id"] + ".wav"), c["noise_snr_db"])


def add_noise(path: str, snr_db: float):
    """White-ish room noise at `snr_db`, fixed seed so every run is the same."""
    with wave.open(path, "rb") as w:
        params = w.getparams()
        x = np.frombuffer(w.readframes(params.nframes), dtype=np.int16).astype(np.float64)
    rng = np.random.default_rng(7)
    noise = rng.normal(0, 1, len(x))
    # A little low-pass so it sounds like a room, not hiss.
    noise = np.convolve(noise, np.ones(4) / 4, mode="same")
    speech_power = np.mean(x[np.abs(x) > 200] ** 2) if np.any(np.abs(x) > 200) else np.mean(x ** 2)
    noise *= np.sqrt(speech_power / (10 ** (snr_db / 10)) / np.mean(noise ** 2))
    y = np.clip(x + noise, -32768, 32767).astype(np.int16)
    with wave.open(path, "wb") as w:
        w.setparams(params)
        w.writeframes(y.tobytes())


def duration(path: str) -> float:
    with wave.open(path, "rb") as w:
        return w.getnframes() / w.getframerate()


def transcribe(cli: str, model: str, wav: str, threads: int, prompt: str | None = None):
    t0 = time.perf_counter()
    args = [cli, "-m", model, "-f", wav, "-nt", "-t", str(threads), "-l", "en"]
    if prompt:
        args += ["--prompt", prompt]
    p = subprocess.run(args, capture_output=True, text=True, encoding="utf-8", errors="replace")
    wall = time.perf_counter() - t0
    text = " ".join(line.strip() for line in p.stdout.splitlines() if line.strip())
    timings = {}
    for name in ("load", "total"):
        m = re.search(rf"{name} time\s*=\s*([\d.]+)\s*ms", p.stderr)
        if m:
            timings[name] = float(m.group(1)) / 1000
    proc = timings["total"] - timings["load"] if {"total", "load"} <= timings.keys() else None
    return text, wall, proc, timings.get("load"), p.returncode


def main():
    args = sys.argv[1:]
    prompt = None
    if args and args[0] == "--prompt":
        prompt, args = args[1], args[2:]
    work, cli, models = args[0], args[1], args[2:]
    threads = min(8, os.cpu_count() or 4)
    clips = json.load(open(os.path.join(HERE, "clips.json"), encoding="utf-8"))["clips"]
    clip_dir = os.path.join(work, "clips")
    if not os.path.isdir(clip_dir):
        make_clips(clips, clip_dir)
    results = {"threads": threads, "prompt": prompt, "models": {}}
    for model in models:
        name = os.path.basename(model)
        # One warm-up so the first clip doesn't pay for the file cache.
        transcribe(cli, model, os.path.join(clip_dir, clips[0]["id"] + ".wav"), threads, prompt)
        rows = []
        for c in clips:
            wav = os.path.join(clip_dir, c["id"] + ".wav")
            text, wall, proc, load, rc = transcribe(cli, model, wav, threads, prompt)
            hyp, ref = norm(text), norm(c["ref"])
            errs, words = wer(ref, hyp)
            lenient_errs, _ = wer(lenient(c["ref"]), lenient(text))
            terms = c.get("terms", [])
            missed = [t for t in terms if not term_hit(hyp, t)]
            row = {
                "id": c["id"], "kind": c["kind"], "audio_s": round(duration(wav), 2), "text": text,
                "wer_errors": errs, "lenient_errors": lenient_errs, "ref_words": words, "terms": len(terms), "terms_missed": missed,
                "wall_s": round(wall, 3), "proc_s": None if proc is None else round(proc, 3), "load_s": load, "rc": rc,
            }
            if c.get("command"):
                row["command"] = c["command"]
                row["command_ok"] = starts_with_command(hyp, c["command"])
            rows.append(row)
            print(f"{name} {c['id']}: wer {errs}/{words} proc {row['proc_s']}s cmd {row.get('command_ok', '-')} missed {missed}", flush=True)
        results["models"][name] = {"size_mb": round(os.path.getsize(model) / 1e6), "rows": rows}
    out = os.path.join(work, "results-prompted.json" if prompt else "results.json")
    with open(out, "w", encoding="utf-8") as f:
        json.dump(results, f, indent=1)
    print("wrote", out)


if __name__ == "__main__":
    random.seed(7)
    main()
