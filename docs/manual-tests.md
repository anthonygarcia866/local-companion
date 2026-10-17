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
2. **No Y/N hints:** look at the card.
   - **See:** no "Y"/"N" hints on the buttons.
   - **See:** there is **no key for Allow**, only a click.
3. **With nothing waiting:** press Ctrl+Alt+Shift+N.
   - **See:** nothing happens except the lantern looking annoyed.
4. **Question cards:** with a question card up (Claude Code asking you to choose), the key does nothing.

**Cleanup:** none.

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
6. **Google Docs (needs screen reader support):** open a Google Doc in Chrome **with screen reader support off** (Tools → Accessibility; Ctrl+Alt+Z toggles it) and type the test sentence.
   - **See:** after ~3 s, Glim shows **"Google Docs: Glim can't read this document"** with "Turn on screen reader support (Ctrl+Alt+Z)…" and **Got it** / **Not now**. glim.log has `writing: Google Docs field not readable (pattern …, read-only …)` and `writing: Google Docs tip offered` (no text).
   - Click **Not now**. Press **Ctrl+Alt+Z** in the Doc, type the test sentence again.
   - **See (to be found out):** whether Glim now gets a count. Tell Claude either way, with the log's `Google Docs field not readable` lines, if any.
   - Quit and restart Glim, go back to a Doc with support off: the tip comes back (Not now isn't a dismissal). Click **Got it**: restart again, and it never comes back (log: `writing: Google Docs tip dismissed`).

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

## After testing

- Tell Claude Code what passed and what didn't. For a failure, say the section and step, and what you saw instead.
- Leave Glim as you use it day to day: not paused, launch at startup as you like, the dock where you want it.
