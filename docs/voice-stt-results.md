# Voice + Actions 1.5a: speech-to-text comparison, first run

Run 2026-10-14 (unattended, for Anthony), per `docs/voice-actions-plan.md` §3. **Read the limits first.** This is a first pass, not the final pick: the clips are synthetic, and the cloud half couldn't be measured.

## What could and couldn't be run

| Candidate | Run? | Why |
|---|---|---|
| whisper.cpp `base.en` (`q5_1`, 60 MB) | **Yes** | local |
| whisper.cpp `small.en` (`q5_1`, 190 MB) | **Yes** | local |
| whisper.cpp `medium.en` (**`q5_0`**, 539 MB) | **Yes** | local. The plan said `q5_1`, but medium.en is only published as `q5_0` (and `q8_0`) |
| **Deepgram Nova-3** (cloud) | **No** | no API key on this machine (env vars and Credential Manager checked by name: only `Glim/AnthropicApiKey`) |
| **AssemblyAI Universal** (cloud) | **No** | no API key either |

**The clips aren't Anthony's voice.** The plan calls for 20 clips of his own speech; none exist yet. These 20 clips follow the plan's mix (dictation with property-management words, command phrases, hard cases) but are spoken by **Windows' built-in offline voices** (Microsoft David and Zira, System.Speech, 16 kHz mono; no network). The mix was changed to **6 dictation, 10 command and 4 hard clips** so that each of the ten approved command words (open / pick up / resume / where was I / draft / email / send / reply / delete / go to) starts one clip.

Synthetic speech is clean, evenly paced, accent-free and has no "um"s, so **real error rates will be higher**, and the gap between models is likely wider on real speech. The latency numbers carry over well, since they depend on clip length and the CPU, not the voice. The accuracy ranking is only indicative.

**Machine:** AMD Ryzen AI 7 350 (8 cores / 16 threads), 15 GB RAM, on AC power. whisper.cpp release `b5454` (2026-10-06), prebuilt `whisper-bin-x64.zip` (CPU), 8 threads. Every download was checked against its published SHA-256 (GitHub's asset digest, Hugging Face's LFS hash). Downloads and clips live in the session's scratch folder, not the repo.

**Tool:** `windows/dev/stt-compare/compare.py` and `clips.json` (dev-only; never ships). Re-run with the owner's clips by swapping `clips.json` and the `clips/` folder.

## How it was scored

- **WER**, two ways:
  - *strict*: lowercase, punctuation, `$`, `,` and `%` removed;
  - *lenient*: strict, plus formatting that isn't a mishearing counted as right ("24th" = "24", "first" = "1st", "9 a.m." = "9am").
  - The lenient figure is the one that matters for dictation, because the Claude clean-up step (§4) normalizes formatting anyway.
- **Command start:** after optional filler ("hey Glim", "um"), the transcript starts with the clip's command word(s). That's Glim's no-Claude "safe to type" rule (§4.1): a miss there means a spoken command could be typed as text.
- **Key terms:** the 47 names, numbers and domain words across the clips ("Okafor", "4B", "HOA", "4471", "dale cuts", "Glim"…), each allowing a few spellings.
- **Speed:**
  - *processing* is whisper's own total minus model load, which is what Glim would pay with the model kept loaded;
  - *wall* is the whole process run, including load.
  - Clips are 1.6–8 s long (median 3.6 s).
- **Two configurations:**
  - *plain*;
  - *with a vocabulary hint*: whisper's `--prompt` given "Glim, HOA, Okafor, Nguyen, Rodriguez Plumbing, dale-cuts, lease renewal, gate code, unit 4B." This is the local counterpart of the cloud services' "keyterms", i.e. a personal word list the user would keep in Settings.

## Results

Run on all 20 clips. Processing times varied between the runs that day (the PC was also busy with other work). The ranges below cover two full runs; the first, under heavier load, was about 1.4× slower across the board.

| | base.en | small.en | medium.en |
|---|---|---|---|
| **Lenient WER, plain** | 2.7% | 3.2% | 2.7% |
| **Lenient WER, with vocabulary hint** | **1.4%** | **0.9%** | **0.9%** |
| Strict WER, plain / hint | 5.4% / 4.1% | 5.9% / 3.6% | 3.6% / 1.8% |
| **Command word heard first** (12 command-start clips) | **12/12** | **12/12** | **12/12** |
| Key terms missed, plain | 4 of 47 | 5 of 47 | 3 of 47 |
| Key terms missed, with hint | 1 of 47 | 2 of 47 | 0 of 47 |
| **Processing, median** (3.6 s clip) | **0.7–1.1 s** | 2.4–3.4 s | 6.6–10.2 s |
| Processing, worst (8 s clip) | 1.0–1.5 s | 2.9–4.7 s | 11.8–13.2 s |
| Model load (once, then kept) | 0.1 s | 0.6 s | 0.6 s |
| Real-time factor (processing ÷ audio) | ~0.2–0.3 | ~0.7–0.9 | ~1.9–2.6 (**slower than real time**) |
| Disk | 60 MB | 190 MB | 539 MB |

**What went wrong, word by word (plain):**
- **All three:**
  - "Glim" → "glimpse" (medium: "Glimm");
  - "HOA board" → "ho aboard" (an email command would go to the wrong place).
- **base:** "Okafor" → "Akafer"; and on the fast clip, "Thursday" → "authors day" (still wrong with the hint).
- **small:** "Okafor" → "Ocopher"; "dale cuts" → "Dale Cutts"; "gate code" → "gatecode".
- **medium:** "Nguyen" → "Wen" on the noisy clip; "dale cuts" → "Dalecuts".

**With the vocabulary hint:**
- "HOA", "Okafor" and "Nguyen" came through in every model.
- "Glim" still came through only for base; small and medium still wrote "glimpse"/"Glimm".
- "Plumbing" → "Plumming" appeared twice, a new slip the hint introduced.

**Commands are safe on every model:** all 12 command clips (the ten command words, plus the fast and noisy ones) started with the right word. Nothing here suggests the §4.1 local rule would let a spoken command be typed as text, on these clips.

## Cloud candidates (prices from the official pages, 2026-10-14; not measured)

Picked on the day as the two that sell short-clip batch transcription and a custom-vocabulary option (needed for names like Okafor, Glim, dale-cuts):

| | Deepgram Nova-3 (English) | AssemblyAI Universal |
|---|---|---|
| Batch (pre-recorded) | $0.0043/min | Universal-2 $0.15/h (= $0.0025/min); Universal-3.5 Pro $0.21/h |
| Streaming | $0.0048/min | Universal-Streaming English $0.15/h |
| Custom vocabulary | Keyterm prompting +$0.0013/min | Keyterms +$0.05/h (batch), +$0.04/h (streaming) |
| At 30 min of dictation a day (≈ 900 min/month), with vocabulary | **≈ $5.0/month** (batch) | **≈ $3.0/month** (Universal-2 batch) |
| Free credit | $200 | $50 |
| Glim's network rules | one new host constant beside `ANTHROPIC_HOST` (plan §6) | same |

Sources: [deepgram.com/pricing](https://deepgram.com/pricing), [assemblyai.com/pricing](https://www.assemblyai.com/pricing). Third-party roundups disagreed with these on several rates; the official pages win.

**Untested here: their accuracy and latency.** A cloud call adds an upload and a round trip. For push-to-talk, where the clip is sent after the key is released, that typically lands in the same range as `small.en` locally, but that's not a measurement. It needs one key and one run of this tool.

## Recommendation

1. **Local default for now: `base.en` with a personal vocabulary list.**
   - It answers a typical utterance in about **0.7–1.1 s** on this PC, well inside the plan's budget (the Claude clean-up step alone has ~4 s).
   - It got every command word right.
   - With the vocabulary hint its lenient WER was 1.4%, the same league as the bigger models on these clips.
   - It's 60 MB, free, and keeps audio on the PC. The vocabulary list ("Glim, HOA, Okafor, …") should be a Settings field in 1.5b; on these clips it removed most name errors.
2. **Don't use `medium.en` on this CPU.** It's the most accurate (0 terms missed with the hint), but it runs **slower than real time** (6–13 s for a 3–8 s clip): push-to-talk would feel broken.
3. **`small.en` is the candidate to re-check on real speech.** On clean synthetic speech it wasn't more accurate than base, at three times the wait (2.4–4.7 s). Real voices usually help bigger models more, so Anthony's own clips may change this.
4. **Before the final pick (the owner chooses, plan §3):**
   - **Record the clips** (exact lines and steps: `docs/voice-clips-to-record.md`; Sound Recorder, kept outside the repo) and re-run `compare.py` on them, about 15 minutes for all three models. That run decides between base and small.
   - **If a cloud option is still wanted:** one Deepgram or AssemblyAI key (both have free credit) and one run. Given these local results, cloud mainly buys accuracy on real, accented or noisy speech and doesn't load the CPU. It costs about $3–5/month at 30 min/day, adds a new allowed host (§6), and sends audio off the PC.
   - Whichever wins, the §4.1 fallback still applies: with a cloud main, a local `base.en` as the automatic fallback is cheap (60 MB, fast).
