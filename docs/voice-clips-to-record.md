# Voice clips to record: base vs small, in your own voice

For Anthony, one sitting of about 15 minutes. These 22 clips replace the synthetic Windows voices of the first run (`docs/voice-stt-results.md`). They settle the one open choice there: whisper **base.en** (fast, ~1 s) or **small.en** (~3× slower, maybe more accurate on a real voice). medium.en is out: it runs slower than real time on this PC.

The mix differs from plan §3 on purpose (8 dictation / 8 commands / 4 hard). It's **6 dictation, 12 commands and 4 hard clips**:
- each of the ten approved command words starts a clip;
- the three `open_app` targets get one clip each, since "Codex" and "Claude Code" are easy to mishear;
- every word the first run got wrong is in here again: Glim, HOA board, Ravenwood, Halloway, Bluefin Plumbing, maple notes, gate code, Thursday on a fast clip.

`windows/dev/stt-compare/clips-owner.json` holds the same list with the reference text and the words that must come through.

## How to record

1. **The app:** Windows **Sound Recorder**, not Glim (Glim never stores audio).
   - In its settings, pick **WAV** if offered and the best quality.
   - Any format works: Claude converts each file to 16 kHz mono WAV with Python's standard library before the run.
2. **The microphone:** use the one you'll dictate with day to day (laptop mic or headset). Same mic, same distance, for every clip.
3. **The room:** a quiet room for everything except **h2**. For h2, turn on real background noise: a TV, a fan or the kitchen.
4. **How to read:**
   - Read each line **exactly as written**, at your normal pace and tone. Don't over-pronounce.
   - Leave about half a second of silence before and after each line.
   - If you stumble, record that clip again.
   - Spoken forms matter: say "nine a.m.", "four two eight six", "seventy-five dollars".
5. **Names:** name each file by its id (`d1`, `c7`, `h3`…). Save them all in **`%USERPROFILE%\glim-clips\`**, outside any repo.
6. **Afterwards:** tell Claude "clips are in glim-clips". The folder is deleted once you've picked a model (plan §3).

## The lines

### Dictation: say these as if writing an email or a note

| id | Say exactly |
|---|---|
| d1 | Hi Priya, the lease renewal for unit 7D is ready to sign. Please send it back by Friday, October 24th. |
| d2 | Um, the HOA board approved the new parking rules, and the fine for a second violation is seventy-five dollars. *(keep the "um")* |
| d3 | The tenant in 9F reported a leak under the kitchen sink. I scheduled Bluefin Plumbing for Thursday at nine a.m. |
| d4 | Thanks for your patience, Mr. Ravenwood. Your security deposit refund was mailed on Monday, and the Section 8 inspection is on the 30th. |
| d5 | I'm testing Glim in Outlook this week, so please ignore any odd drafts from me. |
| d6 | Please remind the vendors that the gate code changes to four two eight six next week. |

### Commands: say these as if talking to Glim

| id | Command word | Say exactly |
|---|---|---|
| c1 | open | Open Outlook. |
| c2 | open | Open Codex. |
| c3 | open | Open Claude Code. |
| c4 | pick up | Pick up from yesterday. |
| c5 | resume | Resume the local companion project. |
| c6 | where was I | Where was I on maple notes? |
| c7 | draft | Draft an email to Priya saying the plumber comes Tuesday at ten. |
| c8 | email | Email the HOA board the October minutes. |
| c9 | send | Send it. |
| c10 | reply | Reply to Elena and say the keys are ready. |
| c11 | delete | Delete the last sentence. |
| c12 | go to | Go to the Glim settings page. |

### Hard cases

| id | What's hard | Say exactly |
|---|---|---|
| h1 | **fast**: say it in a hurry, as fast as you'd really talk | Draft a quick note to the tenants in units 6A through 6F that the water will be off Thursday from ten to two. |
| h2 | **noisy**: real background noise on | Send Mr. Halloway the move-out checklist and copy the property manager. |
| h3 | **pause**: stop for about two seconds where the dots are, as if checking a paper | The repair estimate came to … two thousand four hundred dollars, including parts. |
| h4 | **numbers said two ways** | Call me at five five five, zero one nine eight, about unit four oh two. |

## The run (Claude, about 15 minutes, no Glim build needed)

1. **Convert:** each clip to 16 kHz mono 16-bit WAV in a scratch work dir, as `clips/<id>.wav`. Use Python 3.12's `wave` and `audioop` (standard library), or ffmpeg if it's installed by then.
2. **Set up the work dir:**
   - copy `compare.py`, plus `clips-owner.json` saved as `clips.json`;
   - add whisper-cli (b5454 `whisper-bin-x64.zip`) and the base.en / small.en `q5_1` models, each checked against its published SHA-256 again. Copies are in the 2026-10-14 session scratch folder if it still exists.
3. **Run it twice,** both models each time: plain, then with `--prompt "Glim, HOA, Ravenwood, Halloway, Bluefin Plumbing, maple-notes, lease renewal, gate code, unit 7D, Codex, Claude Code, Section 8"`. That hint list is what you'd keep in Settings → Writing → Dictation → Words Glim should know.
4. **Report:** add a "Run 2 (owner's voice)" section to `docs/voice-stt-results.md` with:
   - lenient WER;
   - command word heard first (12 command clips + h1 + h2);
   - key terms missed;
   - processing time, median and worst.

   Recommend base or small. Then delete `%USERPROFILE%\glim-clips\`.

**How the pick is made:**
- small.en must beat base.en clearly on **your** clips, on names and commands, with the hint on, to be worth ~3× the wait.
- A tie, or a small edge, keeps **base**.
- Any clip where a command word isn't heard first is reported on its own: that's the §4.1 safety rule.
