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
  python -I compare.py [--prompt "<vocabulary hint>"] [--parakeet <model dir>] <work dir> <whisper-cli.exe> <model.bin> [<model.bin> ...]
With --prompt, whisper gets the user's names and words as its initial
prompt (the local counterpart of the cloud services' keyterms).
With --parakeet, NVIDIA's Parakeet TDT 0.6B v3 (int8) runs too, through
sherpa-onnx's Python package (upstream Coucou's Windows speech-to-text,
windows/src-tauri/src/speech/mod.rs there): the dir holds the four files
get_parakeet.py fetches. It is loaded once and kept, so its processing time
is the decode alone, like whisper's "total minus load". Parakeet takes no
vocabulary hint: with --prompt it still runs plain (marked "hint": false).
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


# Parakeet through sherpa-onnx: the files and the version upstream Coucou
# uses (get_parakeet.py has their URLs and SHA-256).
PARAKEET_NAME = "parakeet-tdt-0.6b-v3-int8"
PARAKEET_FILES = ("encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt")
SHERPA_VERSION = "1.13.8"


def read_wav_f32(path: str) -> np.ndarray:
    """A 16 kHz mono 16-bit WAV as float32 samples in [-1, 1] (what sherpa-onnx takes)."""
    with wave.open(path, "rb") as w:
        if (w.getframerate(), w.getnchannels(), w.getsampwidth()) != (16000, 1, 2):
            raise ValueError(f"{path}: needs 16 kHz mono 16-bit, got {w.getframerate()} Hz, {w.getnchannels()} ch, {8 * w.getsampwidth()} bit")
        pcm = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16)
    return pcm.astype(np.float32) / 32768.0


def load_parakeet(model_dir: str, threads: int):
    """The recognizer, loaded once (upstream's settings: nemo_transducer,
    greedy search), and how long the load took."""
    missing = [f for f in PARAKEET_FILES if not os.path.isfile(os.path.join(model_dir, f))]
    if missing:
        sys.exit(f"--parakeet {model_dir}: missing {', '.join(missing)} (run get_parakeet.py)")
    import sherpa_onnx  # only needed with --parakeet

    if getattr(sherpa_onnx, "__version__", "?") != SHERPA_VERSION:
        print(f"note: sherpa-onnx {getattr(sherpa_onnx, '__version__', '?')}, upstream uses {SHERPA_VERSION}", flush=True)
    p = lambda f: os.path.join(model_dir, f)
    t0 = time.perf_counter()
    rec = sherpa_onnx.OfflineRecognizer.from_transducer(
        encoder=p("encoder.int8.onnx"), decoder=p("decoder.int8.onnx"), joiner=p("joiner.int8.onnx"), tokens=p("tokens.txt"),
        num_threads=threads, sample_rate=16000, feature_dim=80, decoding_method="greedy_search", model_type="nemo_transducer",
    )
    return rec, time.perf_counter() - t0


def transcribe_parakeet(rec, wav: str):
    """Same shape as transcribe(): text, wall, proc, load, rc. The model is
    already loaded, so wall = proc = this clip's decode."""
    samples = read_wav_f32(wav)
    t0 = time.perf_counter()
    stream = rec.create_stream()
    stream.accept_waveform(16000, samples)
    rec.decode_stream(stream)
    took = time.perf_counter() - t0
    return stream.result.text.strip(), took, took, None, 0


def score(c, wav, text, wall, proc, load, rc):
    """One clip's row: WER both ways, command start, missed terms, times."""
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
    return row


def run_all(name, clips, clip_dir, one):
    """Scores every clip with `one(wav)` (after one warm-up clip)."""
    one(os.path.join(clip_dir, clips[0]["id"] + ".wav"))
    rows = []
    for c in clips:
        wav = os.path.join(clip_dir, c["id"] + ".wav")
        row = score(c, wav, *one(wav))
        rows.append(row)
        print(f"{name} {c['id']}: wer {row['wer_errors']}/{row['ref_words']} proc {row['proc_s']}s cmd {row.get('command_ok', '-')} missed {row['terms_missed']}", flush=True)
    return rows


def parse_args(argv):
    """--prompt and --parakeet in any order before the positional ones."""
    args, prompt, parakeet = list(argv), None, None
    while args and args[0] in ("--prompt", "--parakeet"):
        if len(args) < 2:
            sys.exit(f"{args[0]} needs a value")
        if args[0] == "--prompt":
            prompt = args[1]
        else:
            parakeet = args[1]
        args = args[2:]
    if len(args) < 2 or (len(args) < 3 and not parakeet):
        sys.exit(__doc__)
    return prompt, parakeet, args[0], args[1], args[2:]


def main():
    prompt, parakeet, work, cli, models = parse_args(sys.argv[1:])
    threads = min(8, os.cpu_count() or 4)
    clips = json.load(open(os.path.join(HERE, "clips.json"), encoding="utf-8"))["clips"]
    clip_dir = os.path.join(work, "clips")
    if not os.path.isdir(clip_dir):
        make_clips(clips, clip_dir)
    results = {"threads": threads, "prompt": prompt, "models": {}}
    for model in models:
        name = os.path.basename(model)
        rows = run_all(name, clips, clip_dir, lambda wav: transcribe(cli, model, wav, threads, prompt))
        results["models"][name] = {"size_mb": round(os.path.getsize(model) / 1e6), "hint": prompt is not None, "rows": rows}
    if parakeet:
        rec, load = load_parakeet(parakeet, threads)
        print(f"{PARAKEET_NAME}: loaded in {load:.1f}s", flush=True)
        rows = run_all(PARAKEET_NAME, clips, clip_dir, lambda wav: transcribe_parakeet(rec, wav))
        size = sum(os.path.getsize(os.path.join(parakeet, f)) for f in PARAKEET_FILES)
        results["models"][PARAKEET_NAME] = {"size_mb": round(size / 1e6), "hint": False, "load_s": round(load, 2), "rows": rows}
    out = os.path.join(work, "results-prompted.json" if prompt else "results.json")
    with open(out, "w", encoding="utf-8") as f:
        json.dump(results, f, indent=1)
    print("wrote", out)


if __name__ == "__main__":
    random.seed(7)
    main()
