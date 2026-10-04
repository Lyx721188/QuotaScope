# Changelog

What each release changed, written for somebody deciding whether to install it.

Windows releases use their own version sequence. Entries below 1.1.1 include
inherited macOS history and do not establish Windows feature availability.

## Unreleased (Windows)

- **Faster local Token views.** Price lookups reuse an indexed snapshot, unchanged transcript caches keep their files untouched, and filtering runs in a cancellable background worker. Cache timestamps retain exact nanoseconds across restarts.
- **DeepSeek console history.** Explicitly imported console sessions provide account history and a balance fallback without an API key. Console credentials and caches are isolated per account; actual charges retain their reported currency, while missing amounts remain unknown.
- **Codex local clues.** The account details show recorded request-setting differences separately from an empirical distribution clue, with sample counts, rule version, cancellation and incomplete-record indicators. These records do not establish which model the server actually ran.
- **Compressed DeepSeek Harness sessions.** The Token page reads real Zstandard frames, including appended frames, with bounded decoding and cancellation. Fork prefixes and duplicate replies are excluded, reasoning is not counted twice, and damaged or incomplete records stay visible through filtering and refresh.
- **Native ZCode usage.** The Token page reads normalized model-attempt counters from ZCode SQLite, including committed WAL data. Retries retain reported usage, cache and reasoning are not counted twice, missing/default counts stay incomplete, and unknown models stay unpriced. Bounded read-only scanning and an in-memory cache support refresh, repair, replacement and deletion without querying conversations. Cache signatures read fixed SQLite/WAL headers and retain WAL commit detection, without sampling arbitrary tail pages.
- **Reliable usage imports.** Standard imports require explicit valid counters and timestamps; incomplete or conflicting records stay partial. Explicit zero snapshots replace earlier usage for the same ID, reasoning is not counted twice, and missing models retain their known tokens as unpriced. All imported sources share directory, file, line, byte and record budgets; JSON arrays stream with cancellation, and exceeding a limit preserves readable counts as partial. See [the import format](Docs/providers/windows-usage-imports.md).
- **Isolated browser Cookie copies.** Read-only fallback copies use a fresh directory for each read, stop on any copy failure, and remove their database and sidecars when the read ends. Profiles with the same directory name cannot overwrite each other's fallback file. See [the Windows read boundary](Docs/providers/windows-browser-cookie-read.md).
- **Bounded history calendars.** Widely separated timestamps retain all recorded counts without allocating every quiet day between them. Recent windows and chart bins follow calendar dates, future records cannot hide today's work, and pricing checks cancellation while constructing dates. See [the calendar rules](Docs/providers/windows-ledger-calendar.md).
- **Validation and limits.** Windows CI checks both the portable Release lifecycle and an independently identified installation of the Release payload, including uninstall cleanup. The actual portable Release executable also passes isolated DSH/ZCode Token page checks. Real DeepSeek/Kiro account verification, upgrading an existing user installation, multi-DPI interaction and interactive login for added accounts remain separate acceptance work. See [the current plan](Docs/release-acceptance-2026-10-04.md).

## 1.3.1 (Windows, 2026-10-03)

- Kiro uses a dedicated native CLI ACP reader with strict, upstream-grounded decoding of credit pools, provider-specific login/version errors, stable pool IDs and deterministic parsing fixtures. Missing limits remain unknown; date-only monthly resets do not imply a fixed duration or a value estimate.
- Each refresh waits for initialization before reading usage, bounds pipe reads and writes, and tears down the hidden CLI and its Windows helper processes on success, failure or timeout. Settings uses Kiro's own saved CLI login and offers enable/refresh controls without storing a Kiro credential.
- Fixture and isolated ACP process tests cover parsing and cleanup. Real Kiro account verification is still pending; see [Kiro Windows](Docs/providers/windows-kiro.md).

## 1.3.0 (Windows, released 2026-10-03)

- **Ten more quota routes.** Windows now reads Kiro, Ollama Cloud, Grok Bot, Volcengine, Devin, Alibaba Token Plan, Gemini, JetBrains AI, Windsurf and Nous Portal. Some routes depend on local CLI state or browser sessions; provider fixtures do not replace verification with a real account.
- **More accounts and useful fallback readings.** Add API-key or session-backed accounts independently. Claude can use an explicitly imported desktop/browser session or status-line cache when its normal route is unavailable. OpenCode adds Go allowance and actual console billing; OpenCode and Kilo can read their local SQLite histories.
- **More detail from local usage.** Claude/Codex views add model share, cache hit rates and available response timing; estimates now handle partial windows proportionally and pause when unexplained external usage is detected. Additional agent logs appear in the spend view, and 20 catalog sources accept the documented import format where native readers are not implemented.
- **Windows controls and integrations.** The panel can dock at the bottom, display horizontal cards and scroll long cards. The tray opens a usage dashboard; optional shortcuts, HTTP proxy settings, deep links, status-line input and CSV export are available.
- **Locale and scan controls.** Traditional Chinese, Japanese and Korean dictionaries are included. Optional background spend scanning uses a bounded cache and can be cancelled when disabled.
- **Known limits.** Interactive OAuth for added accounts, 20 native agent formats, complete BotMark animation and several translations remain incomplete. New provider routes have not all been checked with real accounts. See [the port status](Docs/upstream-implementation-status.md).

## 1.2.4 (Windows, released 2026-10-03)

- Settings keeps its WinUI host and navigation shell, while releasing page controls and the Token spend snapshot on close. Leaving Token spend also releases the snapshot; late scans cannot restore a closed page's data or overlap a new scan.
- Token spend scans can be cancelled, including by closing Settings, leaving the page or disabling local reading. Progress shows the current source, completed sources and files read. Manual refresh checks changed files immediately; cancelling a refresh retains the preceding complete result and does not replace its file cache with partial data. The page's controls and status text now support Simplified Chinese.
- Claude Code and Codex JSONL transcripts now stream one line at a time. Growing Codex logs verify their old prefix and resume saved cumulative counters, including after restarting the app. Rewrites, truncations and unfinished last lines fall back to a full scan; old file caches remain compatible.
- Settings → Storage and diagnostics shows statistics cache size, supports clearing it and offers 16/64/256 MiB disk budgets or disabling persistence. The default is 64 MiB; startup and cache writes evict the oldest rebuildable entries. Account files, original logs and the price table are retained. A local diagnostic JSON export includes process resources and scan status through a field whitelist, excludes credentials and conversation contents, and replaces the previous report.
- The panel uses a slower clock once animations settle, releases its composition canvas when hidden and recreates it when shown. A hidden hover card releases its canvas after one minute. Owned Win32 windows, tray icons and short-lived event handles now have explicit cleanup; the text-format cache is bounded.
- Ledger cache expiry now resets its lifetime before rebuilding, preventing repeated scans after the first five minutes. Expired derived memory is reclaimed during idle maintenance, and the refresh worker waits longer while idle without delaying commands.
- Settings → About adds manual update checks, an opt-in daily check and a per-version reminder skip. Only stable Windows releases with an uploaded Windows package qualify. Checks contact GitHub; downloading and installation remain the user's choice.

## 1.2.3 (Windows, released 2026-10-02)

- **Settings can be reopened safely from the tray.** Closing Settings previously shut down the WinUI host while the tray stayed running. Reopening could then crash the process, including after a long idle. The host now lasts until the tray application exits; closing Settings hides its window and reopening restores it. This release retains the Settings window and controls in memory.
- **Idle tray commands remain responsive.** Reopening Settings, activating the running application from a second launch, and exiting while Settings is hidden no longer depend on account data changing.

## 1.2.2 (Windows)

- **Antigravity's limits are now worth money.** Its detailed card had the day-by-day history but no "Estimated value" line under the windows, because every Antigravity limit is scoped to a model group and the estimator refused all scoped windows. It can now price one: each quarter-hour of the ledger keeps its money split by model, and a window of the Gemini allowance is divided from the Gemini models' spending alone, the Claude-and-GPT window from Claude and GPT. Which group a model belongs to is read from the model's own name (`gemini-3.8-flash-control`, `claude-sonnet-4-6`), which is the guess in this figure — a stretch holding a model whose name says nothing gives up entirely rather than counting the uncertain part as zero.
- **A window is only priced when its own provider can be divided.** Antigravity reports its remaining share to seven decimals where the others report whole percents, so the rounding floor that makes 2% the least usable figure elsewhere is 0.2% here. The other conditions are unchanged and still withhold rather than round down: the logs must start before the window did, and the stretch must hold at least twenty cents.

## 1.2.1 (Windows, released 2026-10-02)

- **Antigravity's own history on this machine.** With Token spend enabled and Antigravity's detailed card switched on, the card reads the conversation databases the IDE keeps under `~/.gemini/antigravity/conversations` and shows per-day tokens and their published API value, the way the Claude Code and Codex cards do. Counts whose meaning has not been established are left out rather than guessed, a turn with no recorded time is filed under the conversation's start, and the card says both. These records show on the account's card only, not in the Token spend overview.
- **A price for the way agents actually write a model name.** The table still comes from models.dev and still refreshes on the first read after 24 hours, but a missing rate was too often a spelling problem rather than a pricing one. The lookup now recognises a provider's own display name (`Gemini 3.7 Flash (High)`), an id that names its vendor (`deepseek/deepseek-v4-flash`, as LM Studio logs write it), and the arm a client appends to the model — Antigravity records `gemini-3.8-flash-control` with the same `model_enum` as `gemini-3.8-flash`, and thinking tokens are billed at the model's own rates.
- **What still has no number.** An experimental build is not a spelling of the model beside it, so `gemini-3.7-flash-exp-b` is counted, and marked, as unpriced — along with the stealth coding-plan models no provider publishes a rate for. A model with no published price is never priced at a neighbouring model's rate.
- Run `cargo run -p quotascope-core --example spend-coverage` to see, per agent on your own machine, which spellings reached a published rate and which did not.

## 1.2.0 (Windows, released 2026-10-01)

- **Correct allowance forecasts.** Millisecond reset times now stay in milliseconds throughout the calculation. A five-hour limit with 39% used after 17% of its time predicts exhaustion in about 80 minutes, rather than incorrectly claiming the allowance lasts until reset. The Chinese copy now says “预计够用至重置” or “预计在重置前用尽”. Predictions assume the cycle's average consumption rate continues.
- **A quieter interface.** Compact usage cards follow the upstream 250-point layout, with three activity summaries and a restrained daily chart. Progress bars use a single capsule to avoid dark seam dots at the ends. Pointer-following accent glow is removed. Settings use aligned rows and account cards with expandable configuration, enabled accounts first and compact inactive summaries.
- **HarmonyOS Sans.** Regular, Medium and Bold fonts ship with the app, including their license; the rail, usage cards and settings load the bundled family without installing system fonts.
- **More provider routes.** The built-in catalog has 77 providers, with 67 Windows routes implemented, including API keys, self-hosted gateways and eighteen browser-session providers. Parser fixtures cover these routes; real-account verification varies by provider. See [the Windows support table](Docs/providers/windows-ports.md).
- **Usage at the tray.** The context menu shows enabled accounts' existing readings, marks stale data and opens the eighteen known usage pages through a submenu.
- **Account settings.** Search and subscription/API groups, a credential visibility switch, per-account balance basis and budget, and a low-balance warning floor. Gateway Save now persists an edited key together with the address and preserves keys that were not edited.
- **Optional detailed cards.** Per-account cards include the plan, last update and reported window clocks. Claude Code/Codex local transcripts can supply token histories and labelled API-value estimates when Token spend is enabled. z.ai and Zhipu statistics use their separate hosts, show thirty-day token history, distinguish empty/missing/failed reads, and never infer a money value.
- **Extensions and display controls.** User-enabled extension programs report limits or balances; configurable warning threshold, clock direction, tray visibility and second-launch settings entry. Read [the Windows guide](Docs/windows-1.2.md) and [extension contract](Docs/extensions.md).

Published as GitHub Release `windows-v1.2.0` (zip, installer and SHA-256).
Interactive UI acceptance on every display is still not claimed.

## 1.1.1

- **The Windows app is now QuotaScope.** The executable, crate names, settings UI, command examples, CI artifact, and GitHub repository all use the new name.

- **Existing Windows data moves forward automatically.** Settings, cached readings, encrypted credentials, and the startup entry are copied or updated on first launch.

## 1.1.0

- **A large balance no longer overflows the ring.** The rail shows ¥5k, ¥123k, $1.2M rather than the full figure, which did not fit and was being cut off — the exact balance is on the card and in Settings. It is always rounded **down**, so the ring never claims you have more than you do.

- **The rail no longer sits slightly too low until you touch something.** On most displays the panel is taller than the space macOS will grant it, and Pulse works out where to draw the rail from the position the window actually got. It was asking that question a moment too early — before the window was on screen, when the answer was still the position it had **requested** — so the rail was drawn about 76pt below where it belonged, and then jumped into place the first time any setting changed.

- **API balances are checked more often.** Pulse paces itself by watching this Mac — an agent writing to its transcripts, a figure that moved, you glancing at the rail — which is why it can be quick when you are working and quiet when you are not. But money spent through an API leaves no trace here, so DeepSeek and Command Code were always being left the full half hour: it waited because nothing had changed, and nothing appeared to change because it waited. Those two are now checked at least every five minutes. Everything else is unaffected, a Mac in low power or with the panel hidden still goes quiet, and a fixed interval you chose yourself still means what it says.

- **The provider list starts in alphabetical order.** It was in the order providers had been added over the months, which meant nothing to anyone reading it. If you have arranged the rail yourself, your arrangement is untouched.

- **Tell me when the credit runs low.** Providers that sell prepaid credit — DeepSeek and Command Code — get a **Warn below** figure in their own settings, and Pulse says so once when the balance falls under it. Money rather than a percentage, because these two report no allowance to take a percentage of; per provider rather than one figure, because ¥20 and $20 are not the same line. Off until you set one, like every other notification here. It is said once and not again until you top up — or until you move the line, which is a new question and gets a new answer.

- **Notifications about a prepaid balance say less, and say it correctly.** Credit that is bought does not reset and cannot be declared spent by arithmetic, so changing what the ring measures against no longer announces a reset, and a balance reaching 100% of a figure **you** set no longer claims the provider says you are out. Only the provider saying so does that.

- **DeepSeek is the seventeenth provider**, and the first one Pulse carries that reports no allowance at all — `GET /user/balance` says how much prepaid credit is left and nothing else. There is no quota, no window and no spend history to read, so the ring needs a denominator from somewhere and you choose which in DeepSeek's settings. **Since top-up** is the default and needs nothing from you: Pulse remembers the highest balance it has watched, and a balance that goes up can only be a top-up, so the ring starts again from full when you add credit. **Balance only** draws no ring at all and puts the money itself on the rail. **My budget** measures against a figure you type. The first two days on "since top-up" will read low — Pulse can only measure from the moment it started watching, and the card says which date that is.

- **智谱's row is now called "Zhipu".** The rail and the settings list are otherwise all Latin script, and one row in Chinese characters read as a different kind of thing rather than as another shop. The company, the storefront and the key it takes are unchanged — this is the name on the row, nothing else. Its sibling stays **z.ai**, which is that company's own spelling.

## 1.0.9

- **Command Code is the sixteenth provider.** It bills a credit balance in dollars rather than a token allowance, so the rings are money: the rolling 5-hour and weekly limits, your organisation's spend limits, and how much of this month's plan is gone. That last one is marked **estimated** on the card, and it is the one thing here Pulse has to infer — Command Code reports what is **left** of a plan's monthly credit but never what the plan grants, which is published on its pricing page instead. A plan Pulse cannot size shows no monthly row at all rather than a reassuring zero. Sign in by pasting a key, or let Pulse borrow the one `cmd auth login` already saved.

- **The panel can follow you between displays.** Switch on "Follow the active display" and the rail moves to whichever screen your pointer is on, keeping the same corner and the same distance down it. There is still only one rail — it is carried across, not copied onto every monitor — and it stays where you last dragged it if you leave the setting off, which is how it ships. "Active" means the display the pointer is on and nothing else: it does not chase other apps' windows around, and it asks for no extra permission to work out where you are.

## 1.0.8

- **The interface follows your Mac's language.** Pulse now declares that it speaks Chinese, which it always did — the translations shipped, macOS just was not told they existed, so a Mac set to 简体中文 got an English app. If that was you, this update switches over on its own; if you preferred it in English, Settings › Language still overrides. The Chinese copy has been rewritten throughout while we were in there.

- **Antigravity can show its two allowances as two rings.** The plan carries one budget for Gemini and a separate one for Claude and GPT, and until now a single ring could only follow whichever was busier — the other went unmentioned unless you hovered. Switch on "A ring for each model group" in Antigravity's settings and each gets its own ring, both under the Antigravity icon, both refreshing the one login. Off by default: an extra ring takes room on the rail, and most people want the one number.

- **智谱 and z.ai get a usage history**, read from the same statistics the console draws its own charts from. Unlike the history Pulse builds for Claude Code and Codex — which it works out by reading session files on this Mac — this one comes from the account, so it covers every machine you use it on. It counts tokens only: the figures behind it cannot be turned into money, and the card says so rather than printing a confident zero.

- **A second limit on the ring.** A thinner ring inside the first shows the next-fullest limit *of the same kind* — the 5-hour beside the weekly it belongs with — so both are readable without hovering. Where a provider splits its allowance by model, the two arcs come from the same allowance wherever it has a second limit to show: pairing one model's weekly with another's 5-hour would put two unrelated budgets on one mark. Off by default and switched on in Settings; the ring is the thing you read without stopping, and two arcs is twice as much to take in. Where a provider reports only one limit nothing is added.

- **The panel no longer slides out from under you as you pick it up.** On a display where the panel is taller than the space under the menu bar — which is most laptops — macOS quietly refuses the position Pulse asks for, and Pulse was then drawing the rail relative to a position the window never had. It jumped about one ring's worth on the first frame of a drag. It tracks the pointer exactly now.

- **The two GLM Coding Plan rows are now named for the shops** — **z.ai** and **智谱**. They were "Z.ai" and "GLM Coding Plan", which was a trap: both shops sell the plan under that same name, so anyone on the international plan picked the row named after their product and had their key sent to the mainland service, which of course refused it.
- **A refused key now says the key was refused.** These services answer with an ordinary HTTP 200 and put the verdict inside, and Pulse only recognised the English wording and two of the numbers — so the most common mistake of all, a key from the other one of the two shops, came out as "the service returned an error" and sent people looking for an outage that was not happening.

## 1.0.7

- **Pulse can tell you, instead of waiting to be looked at.** Three switches in Settings, all off until you turn them on. **Warn at** posts a notification when a limit passes 75, 80, 90 or 95% — whichever you pick — and again when the provider says it is spent. **When a limit comes back** says so once the window you were warned about has turned over, which is the moment you can start again. **When a reading stops arriving** is the one that is about Pulse rather than about usage: a failed check falls back to the last good figures, which is the right thing to show and also the reason the fault is invisible — the panel goes on displaying perfectly plausible numbers with only a "last read" time to give it away. It waits for three failures in a row and then says it once, with the same sentence the card would have shown you.
- **It says each thing once.** A limit already past the line when you switch this on is mentioned straight away — silence followed by a wall is not restraint — and then never again until it resets or gets worse. "Spent" is the provider's own word, never a rounding of ours. A reset is announced only on unambiguous evidence, so a rolling weekly allowance sliding down a few points is not mistaken for a window turning over. And a provider you have never set up, or an app that simply is not running, is not a failure to be reminded of on a timer.
- **Antigravity reads from the IDE too**, not only the desktop app. If Antigravity IDE is the one you have open, its ring said “Open Antigravity to see its usage” while the figures were sitting there for the asking. Both report the same thing — the Gemini and the Claude-and-GPT group, weekly and five-hour each.
- **Antigravity could pick the wrong helper and give up.** It runs more than one of these, only one of them answers, and Pulse asked the first it found and stopped.
- **Reorder the rail by dragging.** The arrows are still there — they are the precise way to move one place, and the only way that works from the keyboard — but with fifteen providers, moving the bottom one to the top was fourteen clicks. There is a Reset order button under the list for when a drag goes somewhere you didn't mean.
- **Volcengine**, bringing it to fifteen. The Ark Coding Plan and Agent Plan, personal and team, each with its five-hour, weekly and monthly windows. Two ways in: `arkcli`, using the login it already saved so there is nothing to paste, or a Volcengine access key pair for anyone who has keys but doesn't run the CLI here. With both set up it prefers the keys — the CLI carries a sign-in that can belong to a different account, and quietly showing the wrong account's limits is worse than either answer.
- **`Pulse --json`**, so the figures can go somewhere other than the panel — a tmux status line, sketchybar, Raycast, a shell prompt. It prints what the app last read rather than fetching, so polling it every second costs nothing and asks no provider anything; every account says when its figures were taken and how old they are. Nothing in the output is translated, so a script parsing it does not break when you change the interface language.
- **Settings has a search field**, since the sidebar now lists fifteen providers plus whatever accounts you have added. It matches the provider's name as well as your own label, so a second Claude subscription you called "work" is still found by typing Claude.
- **The Settings window opens bigger.** It was sized when the sidebar held four rows and had got to the point of appearing already scrolled in both columns.
- Notifications come with the standard notification sound. Silence them, or change anything else about how they arrive, in System Settings › Notifications › Pulse — the same place as every other app.

## 1.0.6

- **Grok**, read from the login Grok Build's CLI already stores — nothing to paste. One thing worth knowing before the ring confuses you: since June 2026 a paid Grok plan spends **one weekly pool across every Grok product** — the web chat, Imagine, voice, the API and the CLI alike — so this is what the account has spent this week, not what the CLI has. That is why it is called Grok rather than Grok Build.
- **Grok Bot**, which is a different limit despite the name. It comes with a Cursor plan rather than a SuperGrok one, so it is read with the login the Cursor editor already stores and carries the xAI mark to tell the two apart on the rail. It appears by itself only if the standalone app is installed; otherwise switch it on in Settings.
- **A second account of either.** Grok signs in with a device code, Grok Bot through Cursor's own sign-in page. Both ask for the narrowest access that can read a limit — never for permission to read or write your conversations.
- A card could print a window length the provider never reported. Some limits carry a length that only exists to sort the rows — a rolling week, a billing cycle — and when there was no reset time to show, that length was printed as though it were one.
- A provider with a single route named the wrong one in Settings. Every such provider but Cursor was described as "Antigravity's language server", about an app it had nothing to do with.

## 1.0.5

- **Claude Code read through the Claude desktop app.** If you work in the desktop app rather than a terminal, Pulse had no way to see your limits: the desktop app hands the CLI a token through its own environment and renews it itself, so the login Pulse was reading went stale and never came back, and it never renders a status line either. Pulse can now read the session the desktop app is signed in with — a new "Desktop app" choice under Read usage from, and the route `Automatic` falls back to once you have allowed it. It asks for the keychain once, at launch, so there is nothing to go and find in Settings.
- **The new route says why it can't answer**, rather than leaving the last reading in place with nothing but its "Last read" time to give it away — which is what makes a refresh look as though it did nothing. It says whether the desktop app is signed out, or whether it was the keychain that was refused.
- **A reading could go backwards.** A newer figure already on file could be replaced on screen by an older one that had just arrived, and a refresh that had been given up on could still overwrite the one that replaced it — including, for an added account, the renewed login itself.
- **GitHub Copilot no longer shows a red ring for paid overage.** Going past the included allowance with overage permitted is not being blocked, and it was being drawn as though it were.
- **Codex no longer marks the wrong model group as spent.** A group reporting "limit reached" could put the mark on another group's window entirely.
- Removing your only added account no longer leaves the rail empty.

## 1.0.4

- **GitHub Copilot**, bringing it to twelve. Signs in with a device code, so there is no token to paste — Pulse asks GitHub for permission to read your profile and nothing else, and never for access to your repositories. Shows the completions, chat and premium-request allowances your plan actually has.
- **Whether a limit will last.** A switch in Settings puts one line under each limit on the card: whether it is on course to outlast its window, and roughly when it runs out if it isn't. Off by default, and it stays quiet when the figures can't carry it — the time only appears when it falls before the reset, and it is rounded, because usage comes in bursts and a figure to the minute would be made up.
- **Show what's left instead of what's spent.** Another switch, which turns the figure and the ring over together so a limit reads "88% left" rather than "12% used". The colour still means how close you are, so a nearly empty ring is still red.
- **Claude Code's card names the plan** — "Max 5x", "Pro", "Team" — as every other provider's already did. The multiplier is part of it, since a Max 5x and a Max 20x are different products.
- **The panel could quietly stop refreshing** after running a long time, and only come back when you next started Claude Code in a terminal. It notices when its own readings have gone stale and asks again, recovers from a fetch that never returned, and refreshes when the Mac wakes as well as when the display does.

## 1.0.3

- **The panel can go on a second display.** Drag it across; it remembers which screen you left it on, and comes home if that screen is unplugged.
- **Four more providers**: the GLM Coding Plan and MiniMax, each with a separate entry for the international and the mainland service, since they are separate accounts with separate keys.
- **A second arc can show how far through the window the clock is**, so "80% used" can be read against how much of the window is left. Off by default, in Settings.
- **The figure can sit above the ring** instead of below it. Also in Settings.
- A provider that needs an API key is no longer switched on by itself — it waits in Settings rather than taking a place on the rail to ask for a key.
- One that has no key says so, instead of saying "Reading…" for ever.
- Claude Code no longer shows a limit that has already reset. If its saved login has expired and no session has run for a while, the stale window is dropped rather than shown with an old reset time.
- The update window now shows the release notes itself, rather than loading the GitHub page inside it.

## 1.0.2

- **Multiple accounts.** Sign in to a second Claude Code or Codex subscription and watch both at once, each with its own ring.
- **Cursor**, reported as the two pools its own account page shows.
- **Ollama Cloud**, from PcOffeeP's pull request — with the session read out of your browser rather than copied by hand.
- **The rail can dock along the top of the screen**, above the menu bar.
- **A colour of your own for any ring**, and the gap between rings is now adjustable.
- **Percentages can be switched off** on either rail.
- The rail opens with the last reading instead of sitting blank.
- The activity mark no longer keeps turning for a minute after a turn has ended.
- The API-key field in Settings lets go when you click away from it.
- A limit you have used never reads as 0% any more.
- Quitting Codex no longer takes Pulse down with it.
- A new app icon, drawn on Apple's icon grid.

## 1.0.1

- **OpenCode Go** and **Kimi Code**, bringing it to five agents.
- **Put the rings in your own order**, in Settings.
- **A refresh button on every provider's pane**, with the age of the reading beside it.
- A new install starts with the agents you actually have, rather than five rings that say "not configured".
- The panel can be dragged by any part of the capsule, not only by its rings.
- Clicking a ring refreshes the one you clicked, whatever order the rail is in.
- The detail card no longer truncates itself at Small or sit half empty at Large.

## 1.0.0

- The first release. A floating rail of rings against the edge of the screen, one per coding agent, showing how much of each limit is left.
