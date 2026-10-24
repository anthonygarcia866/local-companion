# Manual tests

A checklist you can follow by hand, one section per feature, so after a change you test only what changed. Each section says **where the feature is** (which branch or PR), **what to do**, **what you should see**, and **how to clean up**.

Written 2026-10-12. Keep it up to date: every PR that changes what you see adds or edits its section.

## Before you start

- **Know which build is installed.** Features on a branch that isn't merged are only there if that branch was built and installed.
  - Today `main` has everything up to 1c: the checker, fixes, Apply all, the approval queue, a pill per session, the Deny key, and the Claude rewrite (PR #16, 2026-10-13).
  - The `phase-1d` branch adds sections 16–21 below: per-app switches, the one-time prompt, rewrite tones and Casual, strictness, the editable pause list, and the pause key. It also changes section 11 (Pause).
  - Ask Claude Code to build and install a branch; it tells you the build time. The file is `%LOCALAPPDATA%\Glim\glim.exe`.
- **Glim's log** is `%LOCALAPPDATA%\Glim\glim.log`. Several checks below look at its last lines. To read them, open PowerShell and run:

  ```powershell
  Get-Content "$env:LOCALAPPDATA\Glim\glim.log" -Tail 30
  ```

  The log must **never** contain the words you typed. Spot-check that in any section that types text.
- **A scratch Notepad file** is the safest place to type test text. Close it at the end without saving.
- **Test numbers that are safe to type:**
  - card `4111 1111 1111 1111` (a standard test Visa);
  - SSN `123-45-6789`;
  - IBAN `DE89 3704 0044 0532 0130 00`.
  - None of them is real. Never type real ones.
- **A test sentence with two mistakes:** `We recieved the payment and will deposit it tomorow.`

---

## 1. Starting Glim, launch at startup, quitting

*Where: main.*

1. **Start it:** Start menu → **Glim**.
   - **See:** a small pill at the top of the screen. The lantern starts dark, its flame lights, and it settles into a calm glow.
2. **Turn on launch at startup:** right-click the tray icon (notification area, bottom right) → **Settings…** → General → turn **Launch at startup** on.
   - **See:** Task Manager → Startup apps lists **Glim** as Enabled.
3. **Restart the PC.**
   - **See:** Glim appears on its own after you sign in.
   - **See:** the log has a line starting `autostart:` and a later `--- Glim 0.2.0 started ---`.
4. **Quit from the tray:** tray icon → **Quit Glim**.
   - **See:** the pill disappears.
   - **See:** no `glim.exe` in Task Manager → Details.
5. **Quit from the lantern:** start Glim again, right-click the lantern on the pill → **Quit Glim** (the last item).
   - **See:** the same result.

**Cleanup:** start Glim again. Leave launch at startup as you want it.

## 2. Presence: Normal, Ember, Hidden

*Where: main.*

1. **Pick each mode in Settings:** Settings… → General → **Show Glim as** → **Ember (a small glowing dot)**.
   - **See:** the pill becomes a small glowing dot at the edge where the pill was.
2. **Click the ember.**
   - **See:** the pill comes back (mode Normal).
3. **Cycle with the hotkey:** press **Ctrl+Alt+Shift+Space** three times.
   - **See:** Normal → Ember → Hidden (nothing on screen) → Normal.
4. **Bring it back from Hidden:** go to Hidden with the hotkey, then tray icon → **Open Glim**.
   - **See:** the island opens.
5. **Fullscreen hides Glim:** in Chrome press **F11** (or play a video fullscreen).
   - **See:** Glim disappears while it's fullscreen.
   - **See:** press F11 again and Glim comes back. A maximised (not fullscreen) window does **not** hide it.
6. **Pill size:** Settings → General → **Pill size**, try small, medium and large.
   - **See:** the pill changes size at once.

**Cleanup:** set **Show Glim as** and **Pill size** back to what you like.

## 3. Position and dragging

*Where: main. Top-bar dragging is PR #15.*

1. **Drag the closed pill** to the left edge of the screen and let go.
   - **See:** it snaps to the left edge, standing upright.
   - **See:** try top-left, top-right and the right edge; each snaps there.
   - **See:** Settings → General → **Position** shows where it is.
2. **Each display keeps its own dock:** with two monitors, drag the pill to the other screen.
   - **See:** it snaps there, and is remembered for that screen.
3. **Drag from the top bar (PR #15):** open the island (click the lantern), press anywhere in its **top strip** (not on a button), and drag.
   - **See:** the island moves and docks as when dragging the pill.
4. **A drag never presses a button (PR #15):** open a card that has buttons (an approval card is best, section 9). Press on the top strip, drag, and **let go right over a button**.
   - **See:** the island moves, and **the button is not pressed**.

**Cleanup:** drag the pill back to the top centre.

## 4. Hotkeys

*Where: main. Deny is PR #15. Rewrite is the 1c branch.*

1. Open **Settings… → Shortcuts**.
   - **See:** each Glim hotkey with its keys and a switch, and no warning at the top.
   - **See:** a warning names any key that couldn't be registered (another app holds it). Note which, and tell Claude Code.
2. **Try each one:**

   | Keys | What you should see |
   |---|---|
   | Ctrl+Alt+Shift+Space | Normal → Ember → Hidden (section 2) |
   | Ctrl+Alt+Shift+C | The island opens on the chat |
   | Ctrl+Alt+A | Jumps to a waiting approval; with none, the lantern looks annoyed |
   | Ctrl+Alt+T | Opens the focused pill's terminal |
   | Ctrl+Alt+← / → | Previous / next pill |
   | Ctrl+Alt+Shift+N (PR #15) | Denies a waiting permission card (section 10) |
   | Ctrl+Alt+Shift+Y (fix e) | Allows it, under conditions (section 10b) |
   | Ctrl+Alt+Shift+R (1c) | Opens the Rewrite card (section 13) |

3. **Rebind one:** click its key box, press a new combination, and try it.
   - **See:** the new combination works and the old one doesn't.

**Cleanup:** set it back to its default.

## 5. Writing suggestions (the checker)

*Where: main.*

1. **See the count:** in Notepad, type the test sentence, then stop typing.
   - **See:** within about a second, a **2** on the pill next to the lantern. The lantern is amber with a small bounce.
2. **Open the list:** click the lantern.
   - **See:** "2 writing suggestions", each with the word in context, a short explanation and the replacements, in reading order.
3. **Pages:** type a paragraph with 5 or more mistakes.
   - **See:** 3 per page, with **Previous / Next** and "1–3 of N".
4. **The count clears:** fix the mistakes by hand.
   - **See:** the count goes away.
5. **Suggestions belong to their field:** with mistakes showing, click into another app with no text field (File Explorer), then back.
   - **See:** the count stays the whole time.
6. **Ember peek:** switch to **Ember** with mistakes showing.
   - **See:** the ember brightens and shows the count.
   - **See:** click it, and the island opens on the list; close the island and you're back to the ember.
7. **Signatures and quotes aren't checked:** in Outlook, write a reply. Put mistakes only in the quoted message below, or in your signature.
   - **See:** no count for those.
8. **Chat apps are relaxed:** in Slack or Teams type `u coming tmrw? lol idk, i think so`.
   - **See:** no suggestions.
   - **See:** type `recieved` and it is flagged.
9. **Harper's "word ?" message (PR #15):** type `Can you send it ?` (with the space).
   - **See:** the suggestion says **"Space before the question mark."**

**Cleanup:** close Notepad without saving, and discard test drafts.

## 6. Sensitive fields are never read

*Where: main.* In each case, type the test sentence too. **No count** must appear.

1. **Password field:** any sign-in page, type the test sentence in the password box.
   - **See:** no count.
2. **Card or payment fields:** any shop's checkout page with a card form (don't submit anything). Type the test sentence into the name-on-card field.
   - **See:** no count.
3. **Paused sites:** on `paypal.com` or your bank's site, type it into any text box (a search box will do; don't sign in).
   - **See:** no count.
4. **A card number in the text:** in Notepad type `We recieved card 4111 1111 1111 1111 tomorow.`
   - **See:** no count.
   - **See:** do the same with the SSN and the IBAN; no count either.
5. **Unit lists still work:** type `Units 101 102 103 104 need new smoke detecters.`
   - **See:** a count of 1 (the numbers alone don't block it).
6. **Password managers:** in your password manager, type in any field.
   - **See:** no count.
7. **Check the log:** none of the test words, numbers or card digits appear in the log.

**Cleanup:** close everything without saving. Clear any card form you typed into.

## 7. Fixes in Outlook, Word, Gmail (and Notepad)

*Where: main.* Do this in **Outlook (classic)**, **Word**, **Gmail in Chrome** and **Notepad**.

1. **Type the test sentence**, open the list, and click **received**.
   - **See:** the word is fixed in place and the caret goes back where you were.
   - **See:** the note "Fixed. Ctrl+Z in [app] undoes it."
2. **Undo:** press **Ctrl+Z** once.
   - **See:** the typo is back.
3. **Refused after an edit:** with the list open, change the sentence by hand, then click a fix.
   - **See:** nothing is typed, and "The text changed, so nothing was fixed."
4. **Away from the field:** with the list open, click into another app, then look at the list.
   - **See:** the fix buttons are grey, with "Click in the field first."
5. **Outlook's subject line:** a mistake in the Subject gets fixed the same way.
6. **Word with Track Changes on:** the fix shows as a tracked change.
7. **View-only apps:** Edge, new Outlook, Slack, Teams, and chat sites such as claude.ai in Chrome.
   - **See:** suggestions show but have **no buttons**.
8. **Copy buttons:** a field Glim can't type in (some web forms) shows **Copy** buttons.
   - **See:** clicking one puts the word on the clipboard; Ctrl+V pastes it.
9. **Dismiss:** ✕ on the list.
   - **See:** the count goes and stays gone until you edit the text.
10. **The log** has lines like `writing: fix applied (Word)`, and never the words.

**Cleanup:** discard the test drafts in Outlook and Gmail. Close Word and Notepad without saving.

## 8. Apply all (PR #15)

*Where: PR #15.* Do it in **Outlook (classic)**, **Word** and **Gmail in Chrome**.

1. **See the button:** type `We recieved the payment tomorow, posible.` and open the list.
   - **See:** an **Apply all** button next to the title.
   - **See:** hovering it says it uses each suggestion's **first option**, last to first, and that Ctrl+Z undoes one fix per press.
2. **Click Apply all.**
   - **See:** all three are fixed, and the note says **"Fixed 3 of 3. Ctrl+Z in [app] undoes one fix per press."**
3. **Undo one at a time:** press **Ctrl+Z** once.
   - **See:** **one** fix comes back: the last one Glim typed, which is the sentence's first mistake ("received" goes back to "recieved").
   - **See:** press it again; the next one comes back. **Note any app where one Ctrl+Z undoes all three, or something else.** This hasn't been checked yet.
4. **Stopping part-way:** type the sentence again, click **Apply all**, and **at once** click into another app.
   - **See:** it may stop part-way with "Fixed 1 of 3 …" and the reason ("Click in the field first.").
5. **Not offered** with only one suggestion, in view-only apps (Edge, Slack), or for Copy fields.

**Cleanup:** discard the drafts.

## 9. Claude Code agents: approval cards, a pill per session

*Where: main for cards. PR #15 for the queue, pills per session, Open terminal and the usage-limit card.* You need Claude Code with Glim's hooks installed (the Claude Code section at the top of Settings… shows them as installed).

1. **An approval card:** in a terminal, start `claude` in any folder and ask it to run a command that needs permission (e.g. "run `git status`").
   - **See:** Glim shows a permission card with **Allow** and **Deny**.
   - **See:** click **Allow**; Claude Code runs the command.
   - **See:** ask again and click **Deny**; Claude Code says it was denied.
2. **Two requests at once (PR #15):** ask: "Run `git status` and `git log -1` as two separate commands at the same time."
   - **See:** one card shows "1 more waiting".
   - **See:** answer it and the second card comes up.
   - **See:** none go back to the terminal by themselves while you're answering.
3. **Two sessions in one project (PR #15):** open a second terminal in the **same folder** and start `claude` there too.
   - **See:** a second pill named like **"folder · 2"**.
   - **See:** each session's cards and states appear on its own pill. **Open terminal** from each pill opens the right window.
4. **The second pill goes away:** quit the second session (`/exit`).
   - **See:** its pill goes away.
5. **The error card (PR #15):** when Claude Code stops with an error, the card's second button says **Open terminal** (not "Open in n8n") and opens that terminal.
6. **The usage-limit card (PR #15):** this can only be seen when you really hit a Claude limit. When it happens, check:
   - an **amber** card, "Usage limit, resets [time]";
   - **Open terminal** and **OK**, and **no Retry**;
   - after the reset time it says the limit has reset.

**Cleanup:** `/exit` both sessions.

## 10. The Deny key (PR #15)

*Where: PR #15.*

1. **Deny from the keyboard:** get a permission card up (section 9, step 1), then **without clicking Glim**, press **Ctrl+Alt+Shift+N** from the terminal or any app.
   - **See:** the card closes, and Claude Code says the request was denied.
2. **No bare Y/N hints:** look at the card.
   - **See:** no single "Y"/"N" on the buttons; since fix e (2026-10-17) the buttons show the global chords, **Ctrl+Alt+Shift+N** on Deny and **Ctrl+Alt+Shift+Y** on Allow (section 10b).
3. **With nothing waiting:** press Ctrl+Alt+Shift+N.
   - **See:** nothing happens except the lantern looking annoyed.
4. **Question cards:** with a question card up (Claude Code asking you to choose), the key does nothing.

**Cleanup:** none.

## 10b. The Allow key and waiting cards (fix e, redesigned 2026-10-17, PR #33)

*Where: main once #33 is merged.* Ctrl+Alt+Shift+Y was free system-wide and unassigned in Word (checked 2026-10-17). Every permission card takes the key (your "ask" rules no longer matter here); destructive commands take two presses.

1. **Allow from the keyboard:** in Claude Code, ask for something harmless that needs permission (e.g. `npm test` in a repo). With the card open in Glim, **wait 2 seconds**, then press **Ctrl+Alt+Shift+Y** from any app, without clicking Glim.
   - **See:** the card closes and Claude Code runs the command. glim.log: `decide allow (key) req=…`.
2. **Too soon:** get another harmless card and press Ctrl+Alt+Shift+Y **right away** (under 1.5 s).
   - **See:** nothing is allowed; the lantern looks annoyed (glim.log: `allow key refused (too-soon)`). Wait 2 s and press again: allowed.
3. **An "ask"-rule command that isn't destructive** (e.g. `git merge` on a throwaway branch, or `gh pr merge` on a test PR you'd click anyway): wait 2 s, press once.
   - **See:** allowed with one press, like any other card.
4. **Destructive, two presses:** ask for `rm -rf` on a temp folder (or `Remove-Item -Recurse -Force` on one). Wait 2 s, press once.
   - **See:** nothing allowed yet; the card says **Press again to allow · 3**, counting down 3, 2, 1 (glim.log: `allow key armed`). Press again within 3 s: allowed.
5. **The window resets:** another destructive card, press once, then wait **4 s**.
   - **See:** the "Press again" line goes away after ~3 s; a press now is a first press again (the line comes back), a second one allows.
   - **Again and again (bug 2, 2026-10-18):** press, wait 4 s, press, wait 4 s, press, wait 4 s, then press twice: allowed. glim.log never shows `allow key refused (cut-off)` for a command that fits on the card.
6. **Folded card:** fold the card (the chevron), press Ctrl+Alt+Shift+Y.
   - **See:** refused (`not-open`). Open the island on the card again: it needs its own 1.5 s again.
7. **A long command:** one too long to show whole on the card (or cut short with "…").
   - **See:** refused (`cut-off`); clicking still works.
8. **Waiting too long:** get a card and don't answer for 35 s.
   - **See:** after ~30 s the lantern pops, again every ~4 s until you answer. No sound (off by default).
   - Settings → Claude Code → **Soft sound when a permission waits**: on. Another card, wait 35 s: one soft two-note chime at ~30 s, not repeated. Turn it off again if you like.
   - glim.log after answering: `approval waited 34.2 s (allow (click), escalated) req=…`. Every card's wait is logged (a quick one: no "escalated"); never the command.
9. **Settings → Shortcuts:** "Allow the waiting permission" is listed with Ctrl+Alt+Shift+Y and can be changed; the card's Allow button shows the new chord.
10. **Answered in the terminal (bug 1, 2026-10-18):** get a card, answer **No** in Claude Code's terminal instead. Then another, answer **Yes** there.
   - **See:** each time, Glim's card goes within a second; never left up with Deny/Allow. glim.log: `relay gone — answered in the terminal` for the No, and `approval waited … (answered elsewhere)` for both. Repeat a few times: it always clears.

## 11. Pause (tray)

*Where: main hides Glim on Pause. On `phase-1d`, Glim stays and greys out instead; see section 21 for the key.*

1. **Pause:** tray icon → **Pause**.
   - **See (main):** Glim hides.
   - **See (`phase-1d`):** Glim stays where it is: the lantern goes grey (flame out) and a small grey **❚❚** badge sits where the count goes. The tray menu now says **Resume**, and hovering the tray icon says "Glim (paused)".
   - **See:** type the test sentence in Notepad; no count appears anywhere, and nothing is read.
2. **Resume:** tray icon → **Resume** (main: **Pause** again or **Open Glim**).
   - **See:** the lantern lights again, and the sentence gets its count within a second.
   - **See (`phase-1d`):** opening Glim, Settings or the chat no longer lifts Pause; only Resume or the pause key do.

**Cleanup:** make sure Glim isn't left paused.

## 12. The rewrite: setup (1c)

*Where: main (since PR #16).* You need an Anthropic API key:

1. **Get a key:**
   - Sign in at **console.anthropic.com** and add a payment method or credits under **Billing**.
   - Optionally set a monthly spend limit there too.
   - Go to **API Keys → Create key** and copy the key (it starts with `sk-ant-`). Keep it out of chats and documents.
2. **Save it in Glim:** Settings… → **AI (Claude API)**, paste the key, click **Save key**.
   - **See:** the box is replaced by `sk-ant-…` plus the last 4 characters, with a green dot and **Remove**.
3. **The key is in Windows' vault:** open Control Panel → **Credential Manager → Windows Credentials**.
   - **See:** an entry **Glim/AnthropicApiKey**.
4. **The other settings:** **Model** says Claude Haiku 5.5 (default). Set **Monthly cap ($)** to `1`.
   - **See:** **Spend** says "This month: $0 (0 requests)".
5. **The key isn't in the files:** open `%APPDATA%\Glim\settings.json` in Notepad and search for `sk-ant`.
   - **See:** not found. Close without saving.

## 13. The rewrite: using it (1c)

*Where: main (since PR #16).* Do it in **Outlook (classic)**, **Word** and **Gmail in Chrome**.

1. **Open the card:** write a short paragraph, leave the caret in it, and press **Ctrl+Alt+Shift+R**.
   - **See (main):** the **Rewrite this paragraph** card with four tones: Friendlier, More formal, Shorter, Fix grammar.
   - **See (`phase-1d`):** five tones (Casual is new), with this app's default first and ringed; see section 18.
   - **See:** nothing has been sent yet (the lantern isn't in its "sending" look).
2. **Send it:** click **More formal**.
   - **See:** "Rewriting with Claude…", and the lantern switches to its **sending look** (reserved for data leaving your PC) until the answer arrives.
   - **See:** the rewrite appears in about 4–5 seconds or less. **Note how long it takes.**
   - **See:** under it, the model and a cost such as "Claude Haiku 5.5 · about $0.0001".
3. **Use it:** click **Use this**.
   - **See:** your paragraph is replaced, and "Rewritten. Ctrl+Z in [app] undoes it."
   - **See:** **Ctrl+Z** brings your paragraph back.
4. **Refused after an edit:** rewrite again, but **edit the paragraph** before clicking **Use this**.
   - **See:** nothing is typed, and the card says the text changed.
5. **Try another tone:** from a result, click **Try another tone**.
   - **See:** you're back to the tones.
6. **From the suggestion card:** with mistakes showing, open the list and click **Rewrite…**.
   - **See:** the same Rewrite card.
7. **Chat apps:** in Slack or Teams, rewrite a message.
   - **See:** the rewrite shows, **with no Use this or Copy** (chat apps are view-only).
8. **The log** has a line like `rewrite: ok claude-haiku-5-5 in=212 out=187 $0.000115 1.9s`, and **none of your words**.
9. **Spend:** Settings → AI → **Spend**.
   - **See:** the count of requests and a few hundredths of a cent.

**Cleanup:** discard the drafts.

## 14. The rewrite: refusals (1c)

*Where: main (since PR #16).* For each, open the Rewrite card and click a tone.

1. **Sensitive numbers:** a paragraph with the test card number.
   - **See:** **Not rewritten**, and nothing is sent. Because capture never reads a paragraph holding a card, SSN or IBAN, the card says "Click in the paragraph you want rewritten first." and the log says `rewrite: not sent (no-paragraph)` (2026-10-13, live). The "it looks like it has a card…" message is the last-look check before sending, which a live test doesn't reach. Same for the SSN and the IBAN.
   - **See:** Settings → AI → Spend: the request count hasn't gone up.
2. **No key:** Settings → AI → **Remove**, then rewrite.
   - **See:** "Add your Anthropic API key in Settings → AI first." Save the key again afterwards.
3. **The monthly cap:** set **Monthly cap ($)** to `0.0001`, rewrite once (it goes through), then rewrite again.
   - **See:** "This month's spending cap is reached, so nothing more is sent this month."
   - **See:** Settings shows "The cap is reached…".
4. **Paused:** tray → **Pause**, then press Ctrl+Alt+Shift+R and pick a tone.
   - **See:** "Glim is paused."
5. **Local brain:** Settings → AI → **Brain** → Local model, then rewrite.
   - **See:** it says the local model isn't built yet.
6. **Not in a paragraph:** click in File Explorer's address bar (an app that's off), then press the key and pick a tone.
   - **See:** "Click in the paragraph you want rewritten first."
7. **Too long:** a paragraph over 2,000 characters.
   - **See:** "That paragraph is too long to rewrite".

**Cleanup:** cap back to `1` (or what you want), Brain back to **Claude API**, Glim not paused, key saved.

## 15. Network check (any build)

*Where: all builds.* Glim may only talk to this computer, plus `api.anthropic.com` on the 1c branch.

1. **List Glim's connections:** while Glim runs (and, on 1c, while a rewrite is sending), run this in PowerShell. It lists the connections of Glim and its own WebView2 helpers only, not other apps':

   ```powershell
   $root = (Get-Process glim).Id
   $all = Get-CimInstance Win32_Process
   $ids = @($root); $added = $true
   while ($added) { $added = $false; foreach ($p in $all) { if ($ids -contains $p.ParentProcessId -and $ids -notcontains $p.ProcessId) { $ids += $p.ProcessId; $added = $true } } }
   Get-NetTCPConnection -ErrorAction SilentlyContinue | Where-Object { $ids -contains $_.OwningProcess -and $_.RemoteAddress -notin '127.0.0.1','::1','0.0.0.0','::' } | Select-Object RemoteAddress, RemotePort, State
   ```

   - **See (no rewrite running):** nothing listed.
   - **See (during a rewrite):** only connections on port **443**. To check they're Anthropic's, run `Resolve-DnsName api.anthropic.com`; the addresses should match.

**Cleanup:** none.

## 16. Which apps are checked (1d)

*Where: `phase-1d`.* Settings… → **Writing → Apps**.

1. **The list:** every app Glim knows, each with a switch. Outlook, Word, Chrome, Edge, Notepad and the chat apps are on. VS Code, Cursor, Visual Studio and the terminals are off.
   - **See:** chat apps say "chat rules"; Edge, new Outlook and the chat apps say "suggestions only".
2. **Switch one off:** turn **Word** off, then type the test sentence in Word.
   - **See:** no count, within a second of the switch. Any count that was showing for Word goes.
3. **Back on:** turn Word on again and type in it.
   - **See:** the count comes back.
4. **Outlook and Gmail:** do steps 2–3 in Outlook (classic) and Gmail in Chrome.

**Cleanup:** every switch back as it was.

## 17. "Check writing in [app]?" (1d)

*Where: `phase-1d`.* You need an app Glim has never asked about, such as **WordPad** (`wordpad.exe`) or **Acrobat**.

1. **The question:** with Glim in **Normal**, click in a text field of that app and wait 2–3 seconds.
   - **See:** a card "Check writing in **[app's name]**?" with **Yes**, **Not now** and **Never**, and the lantern pops once.
   - **See:** your caret stays in the app; keep typing if you want.
   - **See:** the log has `writing: asking about wordpad.exe` and **no text or window title**.
2. **Not now:** click **Not now**, or wait 30 seconds.
   - **See:** the card goes. Clicking in the app again **today** doesn't ask again.
3. **Yes:** test this on another day, or on a second unknown app. Click **Yes**, then type the test sentence there.
   - **See:** it gets a count (view-only, no fix buttons).
   - **See:** the app shows in Settings → Writing → Apps, switched on.
4. **Never:** on a third unknown app, click **Never**.
   - **See:** it's never asked about again, including after a restart, and it's listed in Settings, switched off.
5. **Ember:** switch Glim to **Ember** and click in a new unknown app.
   - **See:** the ember shows **?**. Click it to get the card.
6. **Hidden and fullscreen:** in **Hidden**, or with a fullscreen app in front, nothing asks.
7. **Never asked:** File Explorer's address bar, the Start menu search, and password managers.
8. **Behind an approval:** if a Claude Code permission card is up, the question waits until you answer it.

**Cleanup:** Settings → Writing → Apps, switch off any test app you said Yes to.

## 18. Rewrite tones and Casual (1d)

*Where: `phase-1d`.* Settings… → **Writing → Rewrite tones**.

1. **The defaults:** Email (Outlook, Gmail) = More formal; Documents (Word) = More formal; Chat apps and chat sites = **Casual**; Everything else = Fix grammar.
2. **First and ringed:** in Outlook, press **Ctrl+Alt+Shift+R**.
   - **See:** **More formal** comes first, with a ring. In Slack or claude.ai it's **Casual**; in Notepad it's **Fix grammar**.
3. **Second press sends:** with the card open on the tones, press **Ctrl+Alt+Shift+R** again.
   - **See:** it sends the ringed tone, exactly as clicking it would.
4. **Casual:** in Word, pick **Casual**.
   - **See:** a relaxed rewrite; **Use this** works as for the other tones.
5. **Change a default:** set Chat apps to **Friendlier**, then press the key in Slack.
   - **See:** Friendlier comes first.
6. **One app:** under **For one app**, pick **Word** and **Shorter**, then **Add**. In Word, **Shorter** comes first; Outlook still has More formal.

**Cleanup:** set the tones back as you like them; the app's own tone has "Its kind's tone" to remove it.

## 19. Strictness (1d)

*Where: `phase-1d`.* Settings… → **Writing → Strictness**. Under the menu, one line per level says what it covers; the chosen level's line is marked. Use this paragraph in Notepad (retest 2026-10-14: the old one had nothing only Full finds):

`Hi Sarah, I recieved the the lease. Their is a few things to do before friday , ok? The plumber can come at 8am in the morning, and the repair is for free of charge.`

1. **Full (default):** everything below, **plus** "am in the morning" and "for free of charge" (wordiness).
2. **Standard:** those two go; still "recieved", "the the", "Their" and the space before the comma.
3. **Spelling only:** only "recieved" and "the the".
4. **Chat:** in Slack, at any level, "u", "lol" and lowercase sentence starts are never flagged.
5. **Known gap, at every level:** `Alex and I was at the unit. There are a few issues that needs fixing.` gives **no** suggestion. That is Harper, not the level; the note under the levels says so (agreement and meaning errors need the AI; for now, Rewrite fixes them).

**Cleanup:** back to **Full** (or what you prefer).

## 20. Paused apps and sites (1d)

*Where: `phase-1d`.* Settings… → **Privacy**.

1. **Built in, locked:** the password managers and the payment and banking sites are listed under "Built in, always paused" with a lock, and have no Remove.
2. **Add an app:** type `notepad.exe`, then **Add**. Type the test sentence in Notepad.
   - **See:** no count; Notepad isn't read at all.
3. **Refused:** try `Notepad`, `C:\Windows\notepad.exe` and `KeePass.exe`.
   - **See:** each is refused, with the reason under the box.
4. **Add a site:** type `mail.google.com`, then **Add**, and write an email in Gmail in Chrome. *(Retest 2026-10-14: Google Docs can't be used for this, see step 6.)*
   - **See:** nothing is checked there. Outlook (and claude.ai) still are.
5. **Remove** both; Notepad gets its count again within a second, and Gmail within a second of typing.
6. **Google Docs is not supported yet (Decision D, 2026-10-17, merged in PR #31):** open a Google Doc in Chrome (screen reader support on or off, it doesn't matter) and click into the page; type the test sentence.
   - **See:** after ~3 s, **"Glim can't read Google Docs yet"** with "Google Docs keeps its text where Glim can't reach it…" and **Got it** / **Not now**. **No count and no suggestions** in Docs, even while typing. glim.log: `writing: Google Docs document, not supported yet: nothing read (…)` and `writing: Google Docs notice offered` (no text).
   - Typing doesn't stop the notice from showing (it did on 2026-10-16).
   - Click **Not now**. Quit and restart Glim, back to the Doc: the notice comes back. Click **Got it**, restart again: it never comes back (log: `writing: Google Docs tip dismissed`).
   - Sheets and Slides get no notice (only Docs documents).

**Cleanup:** remove your test entries.

## 21. The pause key (1d)

*Where: `phase-1d`.* **Ctrl+Alt+Shift+P** pauses and resumes the writing help, from any app.

1. **From Word, Normal:** press it while typing in Word.
   - **See:** the paused look from section 11 (grey lantern, ❚❚, tray says Resume). The count and suggestions go.
   - **See:** Word does nothing of its own with the key. *(Claude couldn't check Word's key table: you were using the PC.)*
2. **Press again:** the lantern pops back, and the paragraph is checked again at once.
3. **Ember:** switch to Ember and press it. The ember goes grey with ❚❚. Press again to resume.
4. **Hidden:** switch to Hidden and press it. Nothing shows, but the tray icon's tooltip says "Glim (paused)". Press again.
5. **Nothing while paused:** paused, type the test sentence, try Ctrl+Alt+Shift+R and a tone. You get no count and "Glim is paused."; the log has no `rewrite: ok` line.
6. **Agents carry on:** paused, let a Claude Code session ask for a permission.
   - **See:** the approval card shows in Glim as usual, instead of going back to the terminal. The ❚❚ badge stays.
7. **Restart:** pause, quit Glim, start it again.
   - **See:** Glim starts **unpaused**.
8. **Settings → Shortcuts** lists **Pause or resume writing help** with Ctrl+Alt+Shift+P, rebindable like the others.

**Cleanup:** Glim not paused.

---

## 22. Settings navigation (1d retest)

*Where: `phase-1d`.* Open Settings…

1. **Sidebar:** on the left: a search box, then General, Presence, Writing (Apps, Tones, Strictness), Privacy, Shortcuts, Claude Code & Agents, About. The page's sections come in the same order.
2. **Jump:** click **Strictness**. The page scrolls to it, and Strictness is marked in the sidebar. Scroll by hand: the mark follows the section on screen.
3. **Keyboard:** Tab to the sidebar, move with Tab, press Enter on **Privacy**: it scrolls there, and the next Tab continues inside Privacy.
4. **Search:** press **Ctrl+F**, type `ember`: only the "Show Glim as" row (Presence) stays. Type `apps`: Writing's whole Apps list shows. Type `zzz`: "No setting matches “zzz”." Press **Esc**: everything is back.
5. **Narrow:** make the window narrow (under ~640 px): the sidebar becomes a bar on top with the sections side by side; clicking one still jumps there and the heading isn't hidden under the bar.
6. **Language:** switch to Français and back: the sidebar and search are translated, and what you typed in the search stays.

## 23. File drags pass Glim by (1d retest)

*Where: `phase-1d`.* Glim running as the pill, top centre.

1. **Into another folder:** drag a file from one Explorer window to another folder, passing the mouse **across the top of the screen near (not onto) the pill** on the way.
   - **See:** Glim doesn't change, no "Drop your files here"; the file lands in the folder.
2. **Into Outlook:** drag a file onto a new email in Outlook (attach it).
   - **See:** it attaches; Glim never opens.
3. **Onto the lantern:** drag a file onto the pill.
   - **See:** the bucket ("Drop your files here", PDF / Images / Code / Docs) appears as you get close; drop it on a target: it is taken as before.
4. **Change your mind:** drag a file onto the pill until the bucket opens, then move away (or press **Esc**).
   - **See:** the bucket folds away at once; dropping somewhere else then works normally.
5. **Switch:** Settings → General → **File drop zone** off. Drag a file onto the pill: the cursor shows "no drop", the bucket never opens. Switch it back on.

## 24. Check with AI (§15.1, draft PR)

*Where: branch `ai-check`.* Needs the Anthropic API key from section 12 and **Brain: Claude API**. Each check costs a fraction of a cent (Haiku).

Use this paragraph in Notepad (it has one spelling mistake, so the suggestions list appears):

`Alex and I was at the unit on Monday, and we recieved the keys. There are a few issues that needs fixing.`

1. **Harper first:** the list shows only "recieved". At its foot, next to the dismiss ×, there is **Check with AI**.
2. **Click Check with AI.**
   - **See:** the lantern turns to the data-leaving state while it runs, the note says "Checking with Claude…", and the button is greyed.
   - **See (in a second or two):** "Alex and I was" and "issues that needs" are added to the list, each with a small **AI** mark, and the note says "Claude found 2 more suggestions." "recieved" is still there, unmarked.
3. **Apply one:** click "Alex and I were". It is typed into Notepad like any fix; **Ctrl+Z** undoes it.
4. **Edit while it runs:** click Check with AI and type a word straight away. **See:** "The paragraph changed while Claude was checking. Check again." and no AI suggestions for the old text.
5. **Nothing to find:** a correct paragraph with one typo (`We recieved the keys on Monday.`), Check with AI: "Claude found nothing more to fix."
6. **Cap:** Settings → AI, set the monthly cap below what's spent, click Check with AI: "This month's spending cap is reached…", nothing sent. Put the cap back.
7. **No key:** (optional) with no key saved: "Add your Anthropic API key in Settings → AI first."
8. **Card and ID numbers:** add `4111 1111 1111 1111` to the paragraph. Glim doesn't read a paragraph holding one, so there is no list and nothing can be sent (and the automatic check doesn't run either). Remove it.
9. **Automatic check (off by default):** Settings → Writing → **Check with AI automatically when I pause typing** is **off**. Turn it on. In Notepad, type `Alex and I was at the unit on Monday.` (no typo) and stop typing.
   - **See:** about 2 s later the lantern delegates briefly and the AI's suggestion appears with its AI mark, though Harper found nothing. It doesn't go again until you change the paragraph.
   - Turn it **off** again: pausing never sends anything; only the button does.
10. **Log:** glim.log has `ai-check: ok issues=2 dropped=0 (click) claude-haiku-5-5 in=… out=… $… …s` (and `(auto)` for step 9): numbers only, never your text. Settings → AI's spend went up a little.

11. **Nothing found, still reachable (decided 2026-10-14):** setting **off**. In Notepad type `Alex and I was at the unit on Monday.` (no typo: no count). Open the island and **click the lantern**.
   - **See:** the list opens with "Nothing to fix here." and **Check with AI** (and Rewrite…). Click it: the AI's suggestion appears with its AI mark.
   - Click into another app's window that Glim doesn't check (e.g. File Explorer's address bar), then click the lantern: the empty list may still open (it stays with the last checked field, like suggestions do); **Check with AI** then says "Click in the paragraph you want checked first."

## 25. The mini lantern beside the field (1e, PR #22)

*Where: `phase-1-bubble`.* Settings → Writing → **Show the mini lantern next to the text field** is **off** by default: check that first, with it off nothing appears beside any field. Turn it **on**.

Use the paragraph from §19 (`Hi Sarah, I recieved the the lease. …`).

1. **Single-line field** (Outlook's subject line, a browser search box): type `teh lease`.
   - **See:** a small lantern, 22 px, just **outside the field's right end**, vertically centred, with an amber count. It never covers the text.
   - Move the window so the field touches the screen's right edge: the lantern moves **inside** the field's right end.
2. **Multi-line body** (Outlook new mail, Notepad): paste the paragraph.
   - **See (since 2026-10-18):** the lantern level with your line, just right of where the line's text ends, and it follows the caret as you type and move to other lines. A line running to the right edge: just above your line (or below, on the first line), never over the caret. Same in Word's document body.
3. **Tall body** (a long Word document or an Outlook body taller than the screen): the lantern sits at the **right edge of the visible part, level with the line you're typing**, and follows that line as you move up and down. Scroll: it hides while the page moves and comes back when it stops.
4. **Click the lantern:**
   - **See:** the popover opens below it (above it near the bottom of the screen) with the same rows as the island's list, 3 a page, "1–3 of N", and **Rewrite…** / **Close** at the bottom. **The caret stays in your text**: type a letter, it goes into the field.
   - Click a replacement: it's typed in (Ctrl+Z undoes it), the count drops.
   - **Rewrite…**: the island's Rewrite card opens, as with Ctrl+Alt+Shift+R; the mini lantern shows the data-leaving look while the rewrite is sent.
   - **Close** closes it. Open it again and move the mouse away for **20 s**: it closes by itself. With the mouse resting on it, it stays.
   - Click into another field: the popover closes.
5. **Hidden when it should be:** Ctrl+Alt+Shift+P (paused), a password field, an app switched off in Settings → Writing → Apps, Settings → Show Glim as **Hidden**, a fullscreen video in front. In each it disappears within a moment; the island's list is unchanged.
6. **The 125% monitor:** drag Outlook to DISPLAY1 (left, 125%): the lantern is a little bigger (28 px) and still placed right.
7. **Turn the setting off:** the lantern disappears at the next reading and never comes back while it's off.
8. **Ignore (both lists, decided 2026-10-15):** with the setting on again, type `We recieved the keys and recieved the deposit.` in Notepad. Open the popover: each row has **Ignore** at its right. Click it on "recieved": the row goes, the badge recounts, and the island's list loses it too. Type `recieved` in another paragraph: not flagged. Do the same from the island's own list with another typo. **Quit Glim and start it again:** "recieved" is flagged again (nothing was saved). glim.log says `writing: ignore ok (1 ignored this run)`, never the word.
9. **Outlook mail body (owner's retest 2026-10-18: it didn't show there):** new mail in classic Outlook, type `We musn't` in the body. Note whether the lantern shows. Either way glim.log now has a `bubble:` line saying why (shown at …, hidden (reason), or no place (reason) with the rectangles); send that line.


## 26. Dictation, push-to-talk (voice 1.5b, merged) and the two keys (Decisions A and B, merged in PR #29)

> **Two keys since PR #29 (Decision A, 2026-10-16):** **Ctrl+Alt+Shift+V dictates only** (always typed, never a command) and **Ctrl+Alt+Shift+A asks Glim only** (never typed). Steps 5, 6 and 6b use **A**. Steps 14-17 are new. Ctrl+Alt+Shift+A was checked 2026-10-17: free system-wide (`RegisterHotKey`) and unassigned in Word's key table.

*Where: branch `voice-dictation`.* Needs, put there by hand (Glim downloads nothing): `whisper-server.exe` and its DLLs from whisper.cpp **b5454** (the Windows x64 zip), and `ggml-base.en-q5_1.bin`, all in `%LOCALAPPDATA%\Glim\whisper\`. Claude clean-up needs the API key from section 12 and **Brain: Claude API**; each utterance costs a fraction of a cent (Haiku). **Ctrl+Alt+Shift+V takes Word's InsertAutoText on that key** (decided 2026-10-14).

> **Pre-checked by Claude, 2026-10-15 (no voice, no Glim rebuild):** the files are in place (whisper.cpp b5454 `whisper-server.exe`, `whisper.dll`, the 11 `ggml*.dll`; `ggml-base.en-q5_1.bin`, SHA-256 matching the GitHub release digest and Hugging Face). `cargo test --lib voice::live -- --ignored --nocapture --test-threads=1` ran Glim's own code against a copy of that folder: no files → "no-whisper"; base found, small reported missing; one server, **listening on 127.0.0.1 only** (netstat), reused for 20 synthetic clips (~0.5-0.75 s each, 1.1 s for the first with the model load); all 10 command clips → the "heard" card under the no-Claude rule; `stop()` kills it. With `GLIM_LIVE_CLAUDE=1`: dictation cleaned up, "open outlook" → an action, "send it" → "Click Send on the draft to send it.", 0.7-1.1 s each, $0.00041 in all. **Not checked yet (needs a build with this branch + your voice and hands):** every numbered step below.

1. **Settings → Writing → Dictation:** Speech model **Base (faster)**; the hint names `ggml-base.en-q5_1.bin` and the folder. Type `Glim, HOA, Okafor` in **Words Glim should know** and click away.
2. **Dictate, cleaned up:** click in Notepad, **hold Ctrl+Alt+Shift+V** (the pill says **Dictating** right of the lantern while you hold it), say "um thanks for the quick turnaround on unit 4B we'll get the lease to you by friday", let go.
   - **See while held:** the lantern turns teal and its flame moves with your voice (s-hear), and a thin teal bar along the pill's bottom edge grows and shrinks with your voice (silent: a sliver). **After:** thinking, then the data-leaving state briefly, then `Thanks for the quick turnaround on unit 4B. We'll get the lease to you by Friday.` is typed at the caret. No card. **Ctrl+Z** undoes it.
2b. **The two sentences that turned into a card on 2026-10-16 (Decision A's regression):** in Notepad, hold **V** and say "This is a test of Glim dictation in my own voice", then "Please send the updated lease to the tenant before Friday afternoon". **See:** each typed as you said it, cleaned up (capital, full stop). **No card, no question.**
2c. **Unit numbers and names (fix d, 2026-10-17):** in Notepad, hold **V** and say "the leak is in unit four B, Gabriela from Orion Housing will check it in AppFolio". **See:** "unit 4B", "Gabriela", "Orion Housing", "AppFolio" spelled right. Settings → Writing → Dictation shows "Glim always knows these too: Claude, Claude Code, Codex, Glim, Gabriela, Orion Housing, AppFolio, unit 4B, unit 12C."
2d. **A misheard app name (fix d):** hold **A** and say "open Claude" a few times, quickly or mumbled. **See:** the Claude app opens each time, even when the transcript says "Claw" or "Clod" (glim.log: `voice: open_app claude_code ok`, or `voice: open_app by a sound-alike`). "Open the app" still asks which one.
3. **Too short:** tap the key (under 0.4 s). Nothing happens, nothing is sent.
4. **No clean-up (raw text):** Settings → AI, set the cap below what's spent. Dictate "the plumber comes tuesday at ten". **See:** the words typed exactly as whisper heard them, a card "Typed in Notepad, not cleaned up" saying the cap is reached, and a grey **~** badge at the lantern's top right (where the count goes) until that card closes (~8 s). The typed text has no marker. Put the cap back.
5. **No clean-up still holds back command words:** with the cap still blocking, hold **V** and say "open outlook". **See:** nothing typed; card "Here's what I heard (couldn't act on it)", "Not typed: without the clean-up, words that start like a command aren't typed…", with **Copy**. Click Copy, paste somewhere: the words. (With the cap back, the same words with V are typed: V means "type this".)
6. **A command with Claude:** cap back to normal, hold **Ctrl+Alt+Shift+A** (the pill says **Ask Glim**) and say "open outlook". **See (branch `voice-open-app`):** classic Outlook opens, no card, nothing typed (on `voice-dictation` alone: the "couldn't act on it" card). Hold A, "open codex": the Codex app opens (Start calls it ChatGPT). Hold A, "open claude code": the Claude app opens. Hold A, "open claude" (branch `voice-open-claude-app`, 4d): the Claude app opens too (the chat app; same app as Claude Code on this PC). No card for either. (Not installed: "Couldn't open … Is it installed?")
6b. **Pick up where you left off, by voice (decided 2026-10-15, branch `voice-resume`):** hold **A** and say "where was I on dale cuts". **See:** nothing typed; the "Pick up where you left off" card (§27) opens with **dale-cuts first**, the other repos after it. "pick up from yesterday" opens it newest first. Nothing opens until you click a row's Open in Claude Code. glim.log: `voice: resume_work (a repo first)`, never the repo's name or your words.
7. **Not in a field:** click the desktop, dictate a sentence. **See:** "couldn't act on it" card, "Click in a text field first to dictate."
8. **Paused:** Ctrl+Alt+Shift+P, dictate in Notepad. **See:** "Not typed", "Glim is paused.", with Copy. Resume.
9. **Card number:** dictate "my card is 4111 1111 1111 1111". **See:** "Nothing was typed" and the "Not sent: … card, ID or bank number" line; nothing sent to Claude.
9b. **Microphone blocked:** Windows Settings → Privacy & security → Microphone → turn off **Let desktop apps access your microphone**, dictate. **See:** "Windows is blocking the microphone, or another app is using it…" with **Open microphone settings**: click it, that Settings page opens. Turn access back on.
10. **No model:** rename the `whisper` folder, dictate. **See:** "Dictation needs the speech model on this PC…". Rename it back.
11. **Hidden:** Ctrl+Alt+Shift+Space until Glim is hidden, dictate into Notepad: the text is typed; a card result (step 7) still shows.
12. **Quit:** tray → Quit Glim. Task Manager: no `whisper-server.exe` left.
13. **Network and log:** while dictating, `netstat -ano` for glim.exe and whisper-server.exe shows only 127.0.0.1 and api.anthropic.com. glim.log has `voice: held …s, transcribed (base) …s, N words, dictate`, `voice: typed applied (cleaned) Notepad` and the like: never your words.
14. **Ask Glim never types:** click in Notepad, hold **A** and say "thanks for the quick turnaround". **See:** nothing typed in Notepad; a "Here's what I heard" card (or a question card). Whatever you say with A, nothing lands in the field.
15. **Answer a question by voice (Decision B):** hold **A** and say "open the app". **See:** a question card (e.g. "Which app: Outlook, Codex or Claude Code?"), "Hold either voice shortcut and say your answer, or type it below.", an answer box, **Send** and **Cancel**. Now hold **V** (yes, the dictation key) and say "codex". **See:** the card stays up and says **Listening for your answer…**, then **Working on your answer…**; then Codex opens. **Nothing is typed** in Notepad.
16. **Answer by typing:** first click in Notepad (or an Outlook message), then ask "open the app" again with A. Click in the card's answer box, type `outlook`, press Enter. **See:** every letter lands in the box, none in the app behind (retest 2026-10-17: they went to Outlook, "the selection is locked"); classic Outlook opens. Ask again, click in the box, then **Cancel**, and type: the letters land back in Notepad without clicking it. The draft card's **Change recipient** search behaves the same.
17. **Cancel:** ask "open the app" again, click **Cancel**. Then hold **V** in Notepad and dictate a sentence. **See:** the sentence is typed (the question no longer waits). A question nobody answers also stops waiting after 2 minutes (the card closes by itself after 60 s).
17b. **"Never mind" (fix c, 2026-10-17):** ask "open the app" again with A, then hold **A** (or V) and say "never mind". **See:** the card closes; no "Is there something else…?" question; nothing opens. Same on the Draft email card while it asks who it goes to, and with "cancel" or "forget it". glim.log: `voice: dismissed (never mind)`, no `voice: cleanup` line for it (nothing went to Claude). With V alone in Notepad (no card up), "never mind" is typed like any dictation.
18. **Settings → Shortcuts:** **Hold to dictate** (Ctrl+Alt+Shift+V) and **Hold to ask Glim** (Ctrl+Alt+Shift+A) are both listed and can be changed like the others.

## 27. Pick up where you left off (resume card, PR #21)

*Where: `voice-resume-reader`.* **How it's shown (decided 2026-10-15):** the tray icon's **Pick up where you left off** (right-click Glim's tray icon); later also by voice ("pick up from yesterday", `resume_work`, on the voice branch). Never by itself. The dev switch `glim.exe --resume-card` (with `GLIM_DEV=1`) still works.

0. **From the tray:** right-click Glim's tray icon: **Pick up where you left off** sits under Weekly recap. Click it: the card below opens. With Glim **Hidden** first (Ctrl+Alt+Shift+Space until it's gone), the pill comes back and the card opens, as Open Glim does. Pause in the tray still says Pause / Resume correctly.

1. **The card:** "Pick up where you left off", with one row per folder under your home folder that has a HANDOFF.md, **the most recently updated first** (local-companion today). Each row: the repo's name, where it stopped (In progress, else Goal), the branch, "Has open questions" if any, and **Open in Claude Code**.
   - **See:** nothing opens by itself: no console, no Claude Code, until you click.
2. **Click Open in Claude Code** on local-companion.
   - **See:** a console window opens in `C:\Users\antho\local-companion` running Claude Code, whose first message is "Read HANDOFF.md first, check it against git status, and continue from In progress." The card says "Opening Claude Code in local-companion…" and closes.
   - glim.log: `resume: 1 read [...]` (names and counts only) and `resume: opened Claude Code in local-companion`. No HANDOFF text in the log.
3. **Not installed:** (optional) if `claude` isn't on PATH or in `~/.local/bin`, the card says "Couldn't open Claude Code in local-companion. Is it installed?" and nothing else happens.
4. **Close** closes the card without opening anything.
5. **A card number in a HANDOFF.md** (in a scratch repo, e.g. `~/resume-test/HANDOFF.md` with `## In progress` holding `4111 1111 1111 1111`): its row says "Part of it is withheld…" and doesn't show that section. Delete the scratch folder after.

## 28. Draft an email (voice 1.5d, merged in PR #27, with the 4c fixes of 2026-10-17)

> **4c (after the 2026-10-16 test, where every draft request came back as a question):** draft requests use **Ctrl+Alt+Shift+A** (Ask Glim). Claude now passes recipients as said ("me", a name, or a spoken address) and never asks for an address (live-checked 2026-10-17: "draft an email to me …" and "… to myself …" → a draft to "me"). "me"/"myself" = Settings → Email drafts → **Your address** (it was **empty** in your settings.json after last night's test: fill it in and reopen Settings to check it stayed). A spoken address ("jordan lee at example dot com") is joined up, then looked up in Outlook's address book: found → filled in; not found → one chip to confirm. While the card asks **who**, hold **either** voice key and say the answer (a name, "the second one", or an address). Never from leasing@ (refused even if Settings names it).

*Where: main (merged in PR #27).* Needs classic Outlook, **open and signed in** (on 2026-10-15 a background start for COM never answered on this PC, see HANDOFF.md), and Settings → Writing → **Email drafts → Your address** = your own address. Glim never sends: every step ends in Outlook's Drafts at most.

0. **The automated part first** (Claude Code runs it, with Outlook open): `GLIM_LIVE_ME=<your address> cargo test --lib outlook_live::dry_run_probe -- --ignored --nocapture --test-threads=1` (read-only: your account found, the address book counted, a first name with 2+ matches, whether Outlook adds your signature), then with `GLIM_LIVE_DRAFTS=1 GLIM_LIVE_AMBIGUOUS=<that name>` `outlook_live::three_test_drafts`: three **[GLIM TEST]** drafts to you, checked in your own Drafts (from your account, to you only, not sent), then deleted from Drafts and Deleted Items. If Outlook shows "A program is trying to access e-mail address information", say what it showed.
1. **To one person:** hold **Ctrl+Alt+Shift+A**: "draft an email to <a colleague with a unique name> saying the plumber comes Tuesday at ten". **See:** the **Draft email** card: To with their name and address, a subject, the body; **Create draft** enabled; the card doesn't close by itself.
2. **Ambiguous name:** "draft an email to <a first name two people share> about the lease". **See:** "Which <name>?" with a chip for each; Create draft greyed until you click one. Click one: Create draft lights up.
2b. **Answer by voice (Decision B):** same again, but instead of clicking, hold **V or A** and say "the second one" (or the person's last name). **See:** the card says **Listening for your answer…** while you hold it, then that person fills the To line. Say something that matches nobody: "No match for that…", and the card still asks.
2c. **A spoken address:** "draft an email to <a colleague's address, spoken: name at domain dot com> saying hello". **See:** in your address book → their name and address on To; not in it → one chip with the address: click it (or say "first") to use it.
3. **Change recipient:** click it; a search box appears (the island takes the keyboard just for this); type part of a name; click a result: it replaces the name. Settings → Email drafts → **Remembered names** shows the pick only after step 5's Create draft.
4. **CC only when said:** "… and cc Sam": a Cc line appears; without "cc" there is none. No attachments ever.
5. **Create draft:** **See:** the card says it's saved and opened; classic Outlook opens the draft (your signature in it if you have one), in **Drafts** of your own account, **not** leasing@. **Send it yourself or delete it.** Nothing was sent by Glim.
6. **"Draft an email to me" / "to myself":** a draft card (not a question) with To = your own address. With Your address empty: the To line says "Add your email address in Settings → Email drafts first."
7. **Cancel** closes the card; nothing is saved.
8. **Card number:** "draft an email to Sam saying the card is 4111 1111 1111 1111": no card; "Not sent: … card, ID or bank number".
9. **No address set:** clear Settings → Email drafts → Your address; Create draft says to add it; nothing saved. Put it back.
9b. **Never leasing@:** (optional) set Your address to the leasing mailbox's address, draft to Sam, Create draft. **See:** "Drafts are never made in the shared leasing mailbox…"; nothing saved. Put your own address back.
10. **Log:** `draft: card N names, M need a pick or a search`, `draft: saved in Drafts and opened (K picks remembered)`: never a name, an address, the subject or the body.
11. **Change the email you just drafted (fix b, 2026-10-17):** right after step 5 (or 6), close the draft's Outlook window (Save is fine), hold **A** and say "edit the email you just drafted and add that the shift starts at nine". **See:** a **Change the draft** card with the subject and the whole new text (your earlier words kept, the change in), **Update draft** and **Cancel**; no recipients on it. Click **Update draft**: the same draft (not a second one) opens in Outlook with the new text above your signature, the signature unchanged; the card says "Changed in Outlook's Drafts and opened there". Nothing sent. Also try "change the email you just drafted to say Wednesday" and "add to that email that parking is in the back".
    - **Cancel** on the card: the draft in Outlook is unchanged.
    - After 10 minutes, or after deleting the draft: "No draft from Glim in the last 10 minutes to change…", nothing changed.
    - glim.log: `voice: edit_draft`, `draft: read the last draft back (ok)`, `voice: edit_draft ok …s`, `draft: change shown on the card`, `draft: changed in Drafts and opened`. Never the subject or the text.

## 29. Calendar glance (branch `calendar-glance`, draft PR, stacked on #27)

*Where: branch `calendar-glance`.* Needs classic Outlook **open** (Glim never starts it for this). Settings → **Calendar** → **Show my next event** is **off** by default: turn it on.

1. **The calendars:** Settings → Calendar lists Outlook's calendars, **one per row**, each with its mailbox beside the name (grey), your default one checked ("(default)"). With Outlook closed it says "Open classic Outlook to choose calendars…" instead.
   - **If the list is empty with Outlook open** (Glim started before Outlook was ready, or Outlook opened later): wait about a minute, or reopen Settings → Calendar (or click **Calendar** in the sidebar). Glim looks again by itself 10 s, 30 s and 60 s after it sees Outlook running, then every minute, and the list fills in. glim.log: `calendar: 0 found (attempt 1), Outlook is running; next look in 10 s`, then `calendar: 9 found (attempt 3)` (your count). Outlook closed: no retries and nothing logged; Glim only watches for Outlook to start (never starts it).
2. **The next event:** have (or make, in Outlook) a meeting starting in the next 10 minutes. **See:** the lantern glows (brighter, glancing eyes) from 10 minutes before. Click the lantern: a card with the title, "9:00 AM – 9:30 AM · in 8 min", the location, **Close**.
   - A Teams or Zoom meeting: a **Join** button. Click it: the meeting opens in Teams/Zoom (or the browser). Nothing else is ever opened from the card.
   - Two meetings at the same time: "+1 more at the same time".
   - Up to 10 minutes after it started: "started 3 min ago", Join still there. Then the next event.
3. **Close:** the card goes; that event doesn't bring it back (the glow stays until it starts).
4. **Lead time:** Settings → Glow before an event → 5 minutes: the glow starts 5 minutes before instead.
   - **Outlook opened after Glim, meeting made 12 minutes ahead (fix 2026-10-24):** quit Outlook, start (or keep) Glim, then open Outlook. Open Settings → Calendar a few times while you wait. Make a meeting in Outlook starting 12 minutes from now. **See:** the lantern glows from 10 minutes before (at most a minute late). glim.log: `calendar: N found (attempt k)`, then a `calendar: … events in the next 24 h …` line whenever the count changes (reads are every minute but logged only on a change).
5. **All-day events:** hidden by default (never "next"). Switch "Hide all-day events" off: they show in the summary as "All day".
6. **Morning summary (off by default):** switch it on. The next morning (after 6:00), the first read shows "Today: N on your calendar" with times and titles (5 at most, "+N more"), once that day. Close: it doesn't come back that day.
7. **Read only:** nothing in Outlook changes: no meeting accepted, declined, moved or edited; no Outlook window opens by itself.
8. **Log:** glim.log has `calendar: N events in the next 24 h (K with a meeting link), next in M min`, `calendar: nothing to show (no-outlook)` when Outlook is closed. `calendar: N found (attempt k)` when Glim finds Outlook's calendars. **Never a title, a location, a calendar name or a link.**
> **Numbering note (2026-10-20):** §29 is used twice in open drafts: #34 (calendar glance) and #40 (your own apps). Whichever merges second becomes §30. §31-§33 below are for #42, #43 and #44, which had no hand test of their own.

## 31. Session pill labels clear the mini lantern (PR #42, cherry-pick of upstream #522)

*Where: a build with PR #42. Needs a Claude Code session in a folder with a long name.*

1. **Long name:** start Claude Code in a folder with a long name (e.g. `my-very-long-project-name-here`) and open the island so its session pills show. **See:** the label starts to the right of the 24 px lantern; no letter sits under it. A name too long for the pill is cut at its right end, not under the lantern.
2. **Short name:** a session in a short folder (e.g. `glim`): the label still reads fine, a little right of centre.
3. **Not touched:** hold Ctrl+Alt+Shift+V: the **Dictating** label beside the pill's lantern looks as before (this PR doesn't change it).

## 32. Open Claude falls back to the claude:// protocol (PR #43, cherry-pick of upstream #417)

*Where: a build with PR #43. The new step only matters for a Store (MSIX) install of the Claude desktop app; the classic installer's path is unchanged.*

1. **Claude running:** with the Claude desktop app open, click **Open Claude** on its pill's card (or the Claude desktop pill). **See:** Claude comes to the front, as before.
2. **Claude closed, classic install:** quit Claude, click **Open Claude** again. **See:** Claude starts, as before.
3. **Store install (only if this PC has one; skip otherwise):** with Claude installed from the Microsoft Store and closed, click **Open Claude**. **See:** Claude opens (before this PR nothing happened). No browser window, no "pick an app" prompt.
4. **Voice is separate:** Ctrl+Alt+Shift+A "open Claude" still opens the Claude app (it uses a different path, `activate_package`).

## 33. Plan usage says the gauge needs the Claude Code CLI (PR #44, adapted from upstream #479)

*Where: a build with PR #44. Nothing to set up.*

1. **The hint:** Settings → Claude Code & Agents → **Plan usage**. **See:** under the Claude text and before its switch: "The numbers come from the Claude Code command-line tool. The Claude desktop app does not report them, so the gauge stays empty if you only use the desktop app."
2. **Another language:** switch Glim's language (e.g. French) and reopen Settings: the hint is translated, not in English.
3. **Spend unaffected:** Settings → AI → Spend still shows Glim's own estimate (it needs neither the desktop app nor the CLI).

## After testing

- Tell Claude Code what passed and what didn't. For a failure, say the section and step, and what you saw instead.
- Leave Glim as you use it day to day: not paused, launch at startup as you like, the dock where you want it.
