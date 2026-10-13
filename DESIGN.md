# Kunsu Product Design System

## Product promise

Kunsu is the decision surface an AI agent uses when work cannot continue without a human answer. It must feel calm while idle and impossible to misunderstand when a decision is due.

The memorable thing: **the agent is waiting for your decision, and the next action is obvious.**

## Product surfaces

Kunsu has three connected surfaces. They are states of one product, not separate visual themes.

1. **Application** — a native macOS inbox for decisions, history, agent connections, and settings. First launch opens onboarding for Claude, Claude Code, Cursor, or a generic prompt-based integration. **Target (2026-10-09):** redesigned in the dub.co design language — see "Application (dub.co language)". It is the one calm, light surface; the forcing surfaces below keep the black system.
2. **Decision capsule** — a compact, draggable floating reminder. It is visible only while actionable questions exist. The menu-bar app remains available at all times. **Target:** becomes the *Edge notch* — a pure-black band that grows out of the display bezel (see "Edge notch").
3. **Decision overlay** — the focused, full-screen answer flow. It expands from the capsule's visual language into a single-question workspace.
4. **Mobile app (Target, 2026-10-10)** — a native iPhone app for unblocking agents from anywhere: decisions, conversations waiting on the user, permission requests, new conversations. See "Mobile app". It uses the black system of the decision surface, not the light application language.

## Aesthetic direction

- **Direction:** Quiet command center.
- **Mix:** 80% Linear-like information discipline, 20% liquid glass.
- **Decoration:** Intentional, sparse. Matte surfaces hold information; glass and specular light appear only on the active surface demanding attention.
- **Mood:** Serious, lucid, native, and controlled. Never mystical, chatty, or “AI magical.”
- **References:** [Thinking Orbs](https://orbs.jakubantalik.com/) for restraint, dark tonal hierarchy, and state motion; Typeless for native desktop navigation density.

## App identity (Current, approved and installed 2026-10-09)

Ray selected the original **鎮定的 K** artwork: a charcoal rounded square, dark metal stem and ivory ceramic arms. Use the supplied image exactly; do not redraw it as a flat mark. The transparent master lives at `assets/brand/kunsu-master.png` (SHA-256 `476b1cd31f845540ebd010642c0bf5ab4d2365c14a7fc36ef3d5ae1ae9025f84`).

The local plugin uses this same mark and the name **Kunsu 軍師**. Its workflow is prepare → preview → submit → human answer → original conversation → result report. Installation and diagnosis show concrete outcomes and recovery actions, never credential values. Draft templates use a decision sentence, observed facts, a recommendation and consequences; journal and retrospective prompts ask for a response without pretending an ordinary FYI is a decision. Plugin interaction rules and maintenance design live in `plugins/kunsu/DESIGN.md`.

Generate native macOS ICNS and required PNG sizes from this master with the Tauri icon packager. The app bundle, browser favicon and local plugin listing share this artwork. Keep the monochrome menu-bar template and Claude/Codex source marks unchanged. The material treatment belongs to the app identity; decision surfaces retain the existing quiet visual system. Small-size and native bundle readback are acceptance gates in `docs/bdd/local-agent-plugin.md`.

## Core design principles

1. **Idle means invisible.** With no actionable question, the floating capsule is hidden. The menu-bar icon and main app remain the retrieval path.
2. **Attention has one visual language.** A monochrome point-orb and a moving specular rim indicate that an agent is waiting. Do not add competing gradients, mascots, or decorative animations. **Target:** the idle reminder becomes the bezel-black Edge notch carrying the source agent's mark. The decision surface replaced the particle agent with the source mark in its header. The source mark is the only colour the notch may carry.
3. **One question, one clear next action.** Overlay screens never become dashboards. Supporting evidence recedes; decision controls remain visible.
4. **Glass is state, not wallpaper.** Use translucency and white rim light on the currently active control or panel only. Background rails and lists stay matte.
5. **Generated content stays in its frame.** Agents provide Markdown and safe HTML for text, and may author an Artifact-style React/HTML evidence canvas for charts and visuals. The canvas runs only inside a sandbox with no network, no IPC, and no ability to submit. Kunsu owns the decision controls, typography of the surface, spacing, and motion; the canvas inherits Kunsu tokens through the template. (Revised 2026-10-08.)
6. **Silence is literal.** While Kunsu Do Not Disturb is active, notification, capsule, and overlay surfaces are absent; only user-invoked application and menu-bar surfaces remain.
7. **Strict, never stuck.** Kunsu is the user's strictest parent (PRD §1.4): it insists on a response but never traps the user. A forced surface appears only when it is fully operable, and Escape always works.

## Typography

- **UI and body:** Geist Sans, with `Noto Sans TC` and `PingFang TC` fallbacks.
- **Display:** Geist Sans at heavier weights. Kunsu is a utility, so the display face remains related to the UI chrome.
- **Data and code:** Geist Mono / SF Mono with tabular numerals.
- **Scale:** 11, 12, 13, 14, 16, 20, 28, 36 px.
- **Line height:** 1.25 for display, 1.45 for controls, 1.65 for long-form decision context.

## Color

Color is restrained and semantic.

- Canvas: `#08090B`
- Recessed canvas: `#0C0E11`
- Surface: `#111318`
- Raised surface: `#171A20`
- Hover surface: `#1D2129`
- Strong text: `#F4F5F7`
- Body text: `#B2B6C0`
- Muted text: `#737984`
- Hairline: `#292D35`
- Bright hairline: `#414752`
- Selection: `#7486E8`
- T0 / danger: `#EF8D86`
- T1 / waiting: `#D9B26A`
- Complete: `#83C494`

Do not use purple gradients. Selection indigo is a small state accent, never a page wash.

## Spacing and geometry

- **Base unit:** 4 px.
- **Primary scale:** 4, 8, 12, 16, 20, 24, 32, 40, 48, 64.
- **Density:** compact in navigation and lists, comfortable in decision content.
- **Radii:** 6 px controls, 10 px rows, 14 px panels, 18 px active decision surfaces, full radius only for the capsule and small status chips.
- **Borders:** one-pixel cool-gray hairlines. Active glass gets a directional white specular segment, never a full bright outline.

## Layout

- Main application: 220 px sidebar, flexible content rail, optional 280 px contextual inspector on wide windows.
- Decision list: rows and rails, not a bento grid.
- Overlay: the Decision surface — one black panel with an evidence | decision split (or a single centred column). See "Decision surface".
- Capsule: a fixed 44 px reminder. It never grows, previews, or shows a tooltip on hover; hover must not alter its geometry or visual state. **Target:** replaced by the Edge notch, whose thickness is fixed but whose length may extend along the edge on hover.

### Decision countdown

The 15-minute cycle is computed in the background from persisted timestamps. Once the decision surface is in front of the user, a countdown has no meaning, so the surface header shows no ring, no `m:ss 後強制詢問`, and no countdown announcement — only `稍後再問` sits at the right (revised 2026-10-09). The only visible countdown is the edge notch's rest text during the advance warning (3 minutes by default; Settings can change it, PRD §27.3): `m:ss 後強制詢問` in the warning colour, and, while a guard defers an overdue question, the reason it is waiting (`時間到了，分享螢幕結束就跳出來` and so on).

- Dismissing an unanswered forced overlay begins a new cycle only after the new timestamps are durably stored.
- Warning state is carried by text in the notch, never by colour alone.

### Do Not Disturb

The application toolbar owns one compact `勿擾模式` control. Opening it reveals four presets (`1 小時`,
`4 小時`, `8 小時`, `24 小時`) plus a custom duration field constrained to 15-minute steps and a hard
24-hour maximum. Focus lands on the first preset; arrow/tab navigation, Escape dismissal, labels, error text,
and activation are fully keyboard and screen-reader accessible.

When active, the control becomes a calm persistent status chip: `勿擾至 18:30 · 剩 2 小時 14 分`.
Its popover replaces duration choices with `提前結束` and an explicit statement that pending decisions are
being retained. Time updates may be minute-granular and must not cause toolbar layout shift. The Settings
surface exposes the same control and status rather than a second implementation.

Activation immediately removes the capsule and overlay without animation and suppresses all macOS
notifications. No confirmation notification is emitted. The main application may still be opened manually;
its inbox shows a non-blocking banner explaining that reminders are paused and when they resume. The menu-bar
menu has a disabled two-line-equivalent summary (`目前模式：勿擾` and `剩餘 2 小時 14 分 · 至 18:30`),
pending count, direct presets for 1/2/4/8/12/24 hours, `提前結束`, and `更多時間…` which opens the app
Settings surface. While active it never reveals question content. Native menu separators keep status, duration
actions, and navigation visually distinct. Reduced-motion mode changes no semantics; high contrast, visible
focus, long localized times, and VoiceOver names are required.

### Edge notch (first version shipped 2026-10-08)

The reminder looks like part of the display: a pure-black band flush with the physical screen edge, the way the MacBook notch merges with the bezel. Interactive reference: [`docs/prototypes/edge-notch.html`](docs/prototypes/edge-notch.html). Acceptance: `docs/bdd/edge-notch.md`.

**Geometry model.** The notch's position is one coordinate `s` along the perimeter of the display inset by `T/2` (the band's centerline). The notch is the segment `[s, s + L]`, swept by a `T × T` square brush and extended off-screen on the outer side. When the segment crosses a corner it becomes an L. Every radius is `min(nominal, available clearance)`, so the outline changes continuously through every corner crossing.

| Token | Value |
|---|---|
| Fill | `#000000` — no border, shadow, blur, or glow |
| Thickness `T` | 32 px, fixed |
| End cap (inner convex corner) | 12 px |
| Shoulder (end meets bezel, inverse fillet) | 7 px |
| L inner (concave) corner | 14 px |
| L outer corner | filled to the display corner (hardware rounding does the rest) |
| Text centerline corner arc | 10 px |
| Source mark | 18 px |
| Text | 12 px Geist Sans / PingFang TC, `#B4BAC4`; warning `#EF8D86`; digits tabular (measured at the widest digit, drawn with `tabular-nums`) |
| Rest length | fits the text: `text + T/2 + 28 px` (9 px half mark + 7 px gap + 12 px tail padding); at a corner add `2r − πr/2` ≈ 4.3 px (r = 10), the length the text centerline loses to the corner arc |
| Hover extension | +88 px |

**Placement.** Default: left edge of the main display, top end 8 px below the menu bar, so the Apple menu stays clickable. The menu bar height is the largest of what the menu bar occupies (`visibleFrame`), the hardware notch (`safeAreaInsets.top`) and the standard menu bar height `max(NSStatusBar.system.thickness, 24 pt)` — so with an auto-hiding menu bar and no hardware notch the default top is still 32 pt and the sliding menu bar never covers the notch. Any edge or any of the four corners is a valid rest position. Position persists as display identity + corner, or display identity + edge + the *head end's* ratio along the edge. Saving the head end (not the centre) means a restart or resolution change puts the mark back where it was even if the text length differs from when it was saved; only the tail follows the current text. Display identity is the CGDirectDisplayID (`NSScreenNumber`) with the display name as a fallback that is used only when exactly one connected display has that name, so two identical monitors are told apart. Older saved values (centre ratio, name only) are still read. A saved value that is not a known corner, or an edge with a head (or legacy centre) ratio in 0–1, is rejected on save and ignored on read (the notch uses the default position). Every save also updates the in-memory position, so a later stage refit restores the latest spot, not the launch-time one. Settings' `回到預設位置` deletes the saved value and the shell then sends its own event (`kunsu://edge_position_reset`); the page drops any save still waiting for its 400 ms debounce, forgets its in-memory position, and moves to the default position at once. A stage re-read (display change, sleep/wake) never counts as a reset: reading no saved value for the same display does not move the notch, because the debounced write may simply not have happened yet, or may have failed.

**What moves the notch.** Only three things: a drag, a change in text length, and Settings' `回到預設位置` (straight back to the default position). A display change or sleep/wake refits the stage and puts the notch back at its remembered spot (see Placement); that is not a move. When the text gets longer or shorter, the head end stays put (top of a vertical arm, left end of a horizontal arm) and only the tail grows or shrinks; a corner L stays centred on its corner. The head staying put has priority, but at rest the notch always lies wholly on the straight edge it was on (or is the centred corner L it already was) — never an off-centre L. If longer text would push the tail past that edge, the whole notch moves back so the tail sits 24 px from the perpendicular screen edge, the same gap a release snap leaves; the head moves only as far as that needs. Restoring a saved position keeps the notch wholly on its edge without adding the gap (if it does not fit, the tail is flush with the screen edge). Digits are tabular, and the countdown's minutes are zero-padded to as many digits as this question's warning needs (from `force_at − warn_at`: a 10-minute warning reads `10:00`, then `09:59`), measured and drawn with the same digit count, so the warning countdown ticking every second never changes the length (PRD FR-03): no per-second growth, shrink or spring. The centre and the length ride the same spring (ω 20 at rest, ω 30 for both while dragging), so the head end stays put on every frame of the animation too. Privacy, queue content and other non-geometry changes never move the notch or cancel hover.

**Drag mechanics.** Pointer velocity is smoothed with an EMA. The cursor position 120 ms ahead is predicted, and the nearest edge to that point is chosen with 40 px hysteresis. Along that edge the notch follows the cursor's projection, keeping the grab offset. The rendered `s` follows the target with a critically damped spring along the shorter perimeter direction (ω = 30 while dragging, 20 when settling; the step is the analytic solution, so it settles within 1 px in 380 ms at any frame rate — the prototype's 17 used Euler integration, which damps differently). It never teleports; crossing a corner always passes through an L. A frame whose time step is zero or negative (the browser's frame timestamp can precede the event that started the animation) does not move at all, so the first frame of a drag never lurches. The drag starts on the edge of the arm under the cursor — the nearest of the edges the notch lies on, with no hysteresis toward the edge holding the centre — and the grab offset is measured on that edge, so grabbing either arm of an L never makes it jump, even right next to the corner or on the short arm a hover turned round the corner: the head moves exactly as far along the perimeter as the cursor moves along that arm. The one exception is the `T × T` square where the two arms of an L overlap at the screen corner: a point there is on both arms, so the press does not pick one. Once the pointer has moved the 4 px drag threshold, the arm that runs in the direction of that movement is picked (horizontal movement → the horizontal arm; a tie goes to the nearer edge). That pick has no hysteresis; the grab offset is measured then, on the picked edge, from the press point, so the head again moves exactly as far as the cursor moves along that arm. A drag that starts while hovering collapses the hover anchored at the head (the tail retracts, the head stays), and the grab offset is measured against that collapsed notch, so the cursor keeps its distance to the head and the head does not jump on the first frame. On release, if the centre is within 30% of the length from a corner, the notch snaps to that corner as an equal-armed L. If it straddles a corner but is further away, it slides fully onto the dominant edge with a 24 px gap. The snap and the final length are decided together: a corner uses the corner rest length (with the arc loss), an edge uses the straight rest length, and the 24 px gap is measured with that final length — a drag that started from a corner L does not keep its corner length and end up 28 px away.

**Reading order.** The source mark sits at the notch's *head*; text runs head → tail.
- Vertical arms read top to bottom. Horizontal arms read left to right.
- In a top-left corner L, the head is the right end of the top arm: text reads leftwards, then down. In a bottom-right corner L, the head is the top of the right arm: text reads down, then leftwards.
- Han characters stay upright. Across the corner arc they lean up to 14° and return upright.
- Latin words rotate 90° on vertical arms, as one unit. A Latin word is a run of half-width visible characters — letters, digits and the symbols between them (`$ , . % - + : / ? = & # ( ) < >`) — plus accented Latin letters; spaces and any full-width character (Han, full-width punctuation, arrows, full-width letters) break it. `NT$5,940`, `-12.5%`, `23:30` and `example.com/r?a=1` are each one word.
- The head end is frozen during a drag, so text never flips mid-gesture. If the settled position prefers the other end, the text at the new end fades in over 180 ms (no slide, no flip, no cross-fade).

**Content states.**
- `你有一則待處理通知` (`你有 N 則待處理通知` when there are several).
- `m:ss 後強制詢問` in the warning colour during the advance warning (3 minutes by default, configurable in Settings). The digits are tabular and the minutes are zero-padded to the digit count of this question's warning length (`09:59` under a 10-minute warning; `2:59` under the default), so the notch keeps one length for the whole countdown.
- While a guard defers an overdue question, the notch names what is blocking it and when it will appear (approved 2026-10-09; replaces `已到期，等待安全時機`): `時間到了，分享螢幕結束就跳出來`, `時間到了，鏡頭關掉就跳出來`, `時間到了，麥克風關掉就跳出來`, `時間到了，離開全螢幕就跳出來`, `時間到了，HH:MM 勿擾結束就跳出來` (Kunsu do-not-disturb, including the global pause shortcut), `時間到了，HH:MM 安靜時段結束就跳出來`. Only the first matching reason in that order is shown; the accessible name lists all of them. Until the first reason arrives after the question becomes overdue, the notch keeps the text it was showing (usually `0:01 後強制詢問`; the ordinary pending text if it loaded already overdue), so `時間到了，馬上跳出來` never flashes first. If the reason cannot be read (an error, or no reply within 2 s), or no guard matches (the overlay is about to appear), it reads `時間到了，馬上跳出來`. The text carries no question content, so it also shows while a privacy guard is active. The Do Not Disturb line is not visible today: Kunsu's own Do Not Disturb hides the notch entirely, and system Focus detection is always false; the line stays in the table for when system Focus is detected.
- While the earliest question is already being asked (`forced`: the overlay shows it, or the main window does after an overlay timeout), the notch shows the ordinary pending text and colour, never `時間到了…`. The overlay covers the notch on its own display anyway; this matters on other displays and during the main-window fallback. The accessible name says `最早一題正在詢問`.
- The state text is recomputed whenever the question list changes, and the next recompute is scheduled at the earlier of 15 s and the earliest question's next `warn_at`/`force_at`, so the warning text appears on time rather than up to 15 s late.
- Nothing else is drawn at rest: no countdown fuse or other line beside the text (removed 2026-10-08 at Ray's request).

**Hover.** After a 120 ms dwell, the notch always grows 88 px from its tail, and the head end (the mark) never moves. If there is not enough room before the next corner, the tail turns the corner along the perimeter and the notch becomes an L (approved 2026-10-09; it no longer grows from the head). The head does not move while it grows or collapses; collapsing is anchored at the head too. The text becomes a marquee of the queued `question_line`s as plain text (Markdown and HTML stripped — only real tags, i.e. `<` followed by a letter, `</` or `<!`; a bare `<` or `>` such as `CPA<200 且 ROAS>3` is content and stays, and an autolink `<https://…>` shows its URL), separated by `·`: it pauses 800 ms, then flows tail → head at 38 px/s and loops. Internal questions (such as the doctor's test question) are neither counted nor played. The notch collapses 260 ms after the pointer leaves. Native code tells the page when the pointer enters or leaves the hit region (and repeats "inside" every second), because WebKit can miss `pointerleave` when click-through toggles; if no "inside" signal arrives for 3 s, hover collapses anyway. Dragging cancels hover immediately. There is never a tooltip.

**Privacy.** While a privacy guard is active (screen share, camera, microphone, frontmost full-screen), the marquee shows only the generic text. Each hover starts private: question text appears only after a fresh guard reading for that hover says it is safe. While the notch is visible the shell pushes guard changes (checked about every 0.5 s, at most 0.75 s apart), and a change during hover takes effect at once. A guard change without hover leaves the rest text alone — it does not fade in again. Known limit: macOS has no public API that reports another app recording the screen or sharing it through ScreenCaptureKit, so a recording without camera, microphone or full-screen is not detected; turning on Do Not Disturb hides the notch entirely.

**Click.** Movement under 4 px opens the decision overlay. The ink transition originates at the source mark's centre.

**Window and hit-testing.**
- **Stage window.** The notch renders in a transparent, borderless stage window covering the full frame of its display, menu bar included. It appears on all Spaces and over full-screen apps. Its level is above the menu bar and below pull-down menus and the overlay. It can never become the key window, and reconciliation only orders it in when it is hidden, so it never takes keyboard focus from the overlay. (A click still activates the app — known gap, AC-EN-15.) When the display configuration changes (resolution, sleep/wake, plugging a display in or out) the stage refits its display within 250 ms and the page re-reads the stage. Whenever the display the stage is fitted to has a different identity from last time — even at the same size, or switched while the stage was hidden — the page is told to re-read too, so it never keeps the previous display's identity or saved position.
- **Click-through.** By default the stage ignores mouse events. A native cursor poll enables them only inside the notch's hit region, which the UI reports whenever geometry changes. The hit region is exactly the drawn shape — the same polygons, shoulders and rounded caps included, with no extra slop — so every visible black pixel is clickable and nothing invisible intercepts. When nothing is drawn (no pending question, only internal ones, or geometry not ready) the UI reports an empty hit region. The poll interval is the cursor's distance to the notch divided by 3000 px/s, clamped to 16–250 ms: any pointer slower than that is switched before it arrives, and polling is at most 10 Hz beyond 300 px.
- **Drag capture.** A press that starts on the notch is latched natively: from the first poll that sees the button go down with the cursor inside the hit region, the stage captures the pointer until every button is released, without waiting for the web layer to report the drag. A drag the web layer reports captures too. Native code ends the capture as soon as no mouse button is pressed, even if the web layer misses `pointerup`. If every question is resolved elsewhere mid-drag, the notch stops drawing: the page treats the drag as released, clears its drag and hover state and reports an empty, non-dragging hit region, and native code drops both the drag and the press latch the moment the hit region is emptied — so a notch that reappears later never captures another app's drag. That release keeps the head where it was drawn: the saved head end is measured with the length the notch had before it vanished, not the shorter empty-queue text. If the stage itself is hidden mid-drag or mid-press (Do Not Disturb, ⌘⇧K), native code drops the drag and the latch and sends `edge_drag_end`, and the page clears up exactly as for an undrawn notch.
- **Why SVG.** The shape is drawn with SVG generated by pure geometry functions. SVG gives crisp arbitrary outlines, per-glyph placement along the curved text path, and `isPointInFill` for tests. Animation stays inside one window, so no window moves or resizes per frame.

**Reduced motion.** Follows both the system setting and Kunsu's own motion setting, live — no restart. Positions and lengths jump to their targets — every target change, including the length collapsing when a drag starts from hover — and velocities reset, so nothing is left animating. The marquee does not scroll; it shows the first `question_line` (plain text) truncated with `…`. All state text and the interactions remain.

**Idle cost.** No animation frames run at rest, reduced motion included (with reduced motion the text fade-in does not run frames either). Without state changes the state text refreshes at most every 15 s (immediately on a question-list change or when `warn_at`/`force_at` arrives; every second during the warning countdown).

### Agent marks (Target)

- **Claude:** the official Claude spark, vector path taken from the Claude desktop app's own `AppLogo`, `#D97757`.
- **Codex:** the Codex blossom cloud with a `>_` prompt, simplified from the Codex app icon. Lavender-to-indigo gradient fill, white prompt strokes.
- **Other agents:** a neutral point mark.

One source drives the decision surface header mark (and its flight from the notch), the static fallback, the Edge notch, and the Agents/Onboarding marks.

### Overlay presentation reliability (Target)

Acceptance: `docs/bdd/overlay-presentation.md`.

- **Frame.** The overlay sets its full frame once, before it is ordered front. The frame is the target display's complete frame in logical points. The target display is the notch's display, else the cursor's display, else the main display. After showing, the shell reads the frame back and re-applies it if it is off by more than 0.5 pt. It also realigns on display reconfiguration.
- **Ready handshake.** The overlay is never ordered front before its webview acknowledges, for the current transition id, that the question, composer, and close control are mounted. Transition events go only to the overlay window. The webview also pulls the pending transition on mount, so an event can never be lost to load timing.
- **Fail-open.** With no acknowledgement within 1.5 s, the overlay stays hidden and the main window opens on that question. The question keeps its force eligibility, and the overlay webview reloads.
- **Escape.** Escape and the close button work in every overlay state. If the webview does not handle Escape within 2 s, the native layer performs the same dismissal. When `稍後再問` fails, the panel stays open with a readable error (`role="alert"`) and can be retried; the webview tells the native layer it handled Escape only after the dismissal succeeded, so a failed Escape still ends in the native dismissal after 2 s and the overlay never gets stuck. The error sits in its own row below the header and pushes the content down; it never covers the question. Within one overlay session each question is dismissed once: a retry only redoes the questions that failed (an already-closed question counts as done), and the native fallback skips questions the webview has already dismissed or is dismissing, even when the webview takes longer than 2 s.
- **Origin without a click.** A forced question or the tray opens the overlay without a click. The opening then starts from the last source-mark centre the notch reported, never from a fixed corner.
- **Session.** A question that becomes forced while the overlay is open joins the end of the session (`第 N 題，共 M 題` grows); it never needs a close and reopen. A question in the session that another channel closes (Discord, CLI, the agent, expiry) turns read-only in place. The last receipt stays on screen only while the panel is showing a receipt answered in this session; when every question in the session has been closed elsewhere and no such receipt is showing, the overlay hides. The decision to hide never runs ahead of an answer that is still being sent: the server announces the list change before it returns the answer, so while an answer is in flight the overlay stays, and during the 300 ms grace before hiding the panel stays mounted and can still claim its receipt. After the overlay hides, the shell checks the forced questions again and presents the overlay again if one is still forced. The main window's answer flow follows the same live list.

### Conversation handoff receipt

The receipt's handoff line names where the answer went:
- `已交給等待中的原對話`
- `正在原對話追加處理…` (pulsing, while the append runs)
- `已在原對話追加處理`
- `原對話正開著，等它空下來就接續`
- `已開新對話處理（原因）`

When the question has a conversation identity, a quiet `打開原對話` text button deep-links into the Claude app (`claude://code/continue?session=…`) or the Codex app (`codex://threads/…`). Because the overlay sits above every window, anything the overlay opens outside Kunsu (the conversation, `轉討論`, Kongko, a link in the body, the AI feedback, an evidence line or the timeline) first steps the overlay aside — unanswered: the same as `稍後再問`; answered: close — and opens the target only after the panel has closed and the window is hidden, so the target never opens hidden behind it. If stepping aside fails, the target is not opened and the panel shows the error. AI feedback from `ask_report` renders below it as sanitized Markdown in the long-form context style. It is never an input field.

### Decision surface

Replaced the split particle-actor overlay (2026-10-08). Opening starts from the notch's source mark (the click point, or the last mark centre the notch reported when there is no click). Acceptance: `docs/bdd/decision-surface.md`; reference: `docs/prototypes/decision-surface.html`.

- **Opening.** The notch extends along its edge to the panel's full length (critically damped, about 300 ms), then spreads into the screen (about 480 ms, no overshoot). Outside the panel the screen stays fully transparent at every moment — no dimming scrim and no blur layer — so the only thing that appears is the black panel spreading from the notch (revised 2026-10-09). The source mark flies to the header, and content fades in with 45 ms staggers (opacity only, during the opening only; a question change never replays it). Closing plays in reverse from wherever the shape is; when nothing is pending, the notch retracts into the bezel.
- **Settling.** Until the shape settles (about 0.93 s) the panel is `inert`: nothing in it can be clicked or focused. Escape works from the moment the panel mounts. If the animation loop does not run, a timer settles the panel within 1.2 s and finishes a close within 0.9 s, so the panel never stays half open.
- **Reduced motion.** Either the macOS setting or Kunsu's own `連續轉場動畫` switch turns off the shape and the mark flight. The panel then cross-fades in and out over 160 ms; the global reduced-motion rule must not shorten this fade.
- **Panel.** Pure black (`#000`), flush with the notch's edge, inset 14 px elsewhere, 28 px inner radius, bezel shoulders. Without the shape (reduced motion, main window), only the two corners facing into the screen are rounded: a left notch rounds the right corners, a right notch the left corners, a top notch the bottom corners, a bottom notch the top corners. The surface always declares `color-scheme: dark` itself, so native controls (the note field's resize handle, spelling marks, autofill, unstyled scrollbars) stay dark even when the application window around it is light. Opened inside the main window, the stage behind the panel is black (`#000`), so no light application edge shows around the panel — the same black-on-black look as the overlay; in the overlay the stage stays fully transparent (revised 2026-10-09, application QA round 1).
- **Type.** The surface uses the application's font restriction: Geist covers only Latin letters, digits and ASCII punctuation, and Chinese text and full-width punctuation (`，。；：「」`) fall through to PingFang TC, so `第 1 題，共 8 題` keeps its full-width comma (revised 2026-10-09; the bundled Geist carried narrow full-width punctuation).
- **Header (68 px).** The left side holds the source mark (26 px), agent name, task chip (`任務 · 排程/互動`), and asked time. Next to it are the queue position (`第 N 題，共 M 題`, dots, ‹ ›). The right side holds only `稍後再問`, with its consequence written out in muted text (`N 分鐘後再問`, where N is the force cycle in effect — read from the settings when the panel mounts, 15 when they cannot be read) and the `Esc` key — visible text, not a tooltip. No countdown (see "Decision countdown"). When reviewing a closed question from history the right side holds `關閉 Esc` instead: there is nothing to ask again (revised 2026-10-09).
- **Body.** With evidence: evidence column (1.42fr, scrolls on its own) | decision column (min 430 px). Without evidence: a single centred column, max 760 px. When the panel itself is narrower than 1100 px, the split becomes one column (evidence, then decision); the evidence takes its full height and the whole body scrolls as one. The rule is a container query on the panel's own width, so the overlay, the 980 px main window and the batch review all switch at exactly the same width (revised 2026-10-09; it used to compare the window width, and the batch review used ≤1100 px). Long unbroken text such as URLs wraps; the decision column never scrolls sideways.
- **Decision column.**
  - `要你決定` label (`要你回應` for text questions), then the 27 px question.
  - Consequence and deadline line, then (money questions) the `所見即所簽` params block.
  - Option cards: number key, label, `AI 建議` badge, detail, tags. Selected cards get a white ring and a filled key. The clickable surface is a radio button stretched under the card; the label and detail sit on top of it and let clicks through, except their links. A link in a label or detail is therefore never inside the button: clicking it runs the link's own two-step open and never selects or clears the option, and Tab reaches it. From the keyboard the first Enter shows the hint and keeps focus on the link; the second Enter opens it.
  - Note field, then the decision summary.
  - Footer: `這題不該問我` (left, text) and the primary action (right, white pill, disabled until ready). Its label follows the question: `送出決定` (choice), `送出回答` (text, or no selectable options), `確認` (confirm without options), `批准並執行` (a yellow money question with its AI-recommended executing option selected), `拍板此選項（不執行）` (a money question whose one-click path is locked), `送出中…` while sending.
  - The footer is always visible and nothing scrolls out beneath it: in the split layout it sticks to the bottom of the decision column with its own bottom padding; in one column (panel narrower than 1100 px) it is pinned to the panel bottom and the body reserves its height.
- **Keyboard and focus.** The panel is a modal dialog: Tab and Shift+Tab cycle inside it. Enter on an option sends the current explicit selection and never changes it; Enter on any other button or link only activates that control. After sending, focus moves to the receipt's `下一題`／`完成` button (batches too); in a receipt, Enter on a link or another button only activates that control.
- **Why card.** Summary, auto timeline (lines starting with `HH:MM` or `現在`; the last item highlighted), `打開原對話`, and `看 AI 送來的原文` with a line and character count.
- **Receipt.** Shown in place: the decision and note, the received / ledger / handoff rows revealed in sequence, and `下一題 ⏎` or `完成 ⏎`. The question and option label render as Markdown, exactly as while deciding (the execution-failed view and the batch receipt too). A confirm question's receipt reads `確認`. In the overlay, the receipt of the last question stays on screen until `完成 ⏎` or Escape; it never vanishes on its own.
- **Money follow-up in the receipt.** An answered money question can need one more step from the user. What it offers follows only the server's `money_state` (never Kunsu's own reading of the execution text; old data without the field falls back to the text):
  - `unknown` — the executor used the authorization but never reported back. Explanation line plus two equal buttons, `我核對過：錢已經動了` and `我核對過：錢沒有動`.
  - `reauthorizable` — blocked, interrupted before the executor verified, or confirmed unmoved. One button, `重新授權並執行`.
  - `running` / `settled` — no button.
  - Every button opens a confirm row first (`確定` primary, `取消`), each with one plain sentence saying what happens next. The row takes focus on `確定`; `取消` returns focus to the button that opened it; after a successful send focus moves to the receipt's `完成`／`下一題`. While a send is in flight both confirm buttons are disabled.
  - The buttons follow the latest question (receipt polling, list updates): a fresh prop always wins, and only the just-sent state is kept locally.
  - The receipt, the execution-failed view and history review all show these buttons.
  - The execution-failed view words itself by state. `reauthorizable` says `修好原因後可以在這裡重新授權`. `unknown` uses the label `結果不明` and says `executor 已用掉授權但沒有回報結果，錢可能已經動過，請先到廣告後台核對再對帳`. When a live execution `fail` event arrives, the column re-reads the question; if it is `unknown` it shows the receipt with the reconcile buttons instead of staying on `執行沒有完成`.
  - The money row's icon carries the state, and the text always says it too: executed ✓, dry run or not executed –, unknown !, still running ….
- **Closed elsewhere.** A question in the session that another channel closes turns read-only in place: the label reads `已在其他地方結案`, a line says what happened (`已由 Discord 回答`, `出題的 AI 已取消`, `已過期`, …), there are no answer controls, and the footer offers `下一題`／`完成`. Its queue dot's accessible name says the same (`第 N 題（已結案：已由 Discord 回答）`), never a blanket `已回答`.
- **Review.** Opening an already-closed question from history is always read-only (the header offers `關閉 Esc`, never `稍後再問`), and the status follows the real outcome: an answered question shows its receipt; an expired one reads `已過期` with `沒有回答` (or `沒有回答（逾時依預設：…）`); one the agent cancelled reads `AI 取消了這題`; one the user rejected reads `已退回重擬` with `你退回了這題：reason`; a superseded one reads `被新題取代`. `已退回重擬` is reserved for the user's own rejection.
- **Question change.** The old content fades out over 140 ms and the new content fades in over 140 ms (opacity only; the opening shape is not replayed). With reduced motion the content swaps at once.

### Evidence canvas

- **Frame.** A card frame (`surface-card`) with a quiet `AI 提供的資料` caption and a small accent dot, placed first in the evidence column. The iframe background is transparent, and its height follows the content via `resize` messages (max 2400 px). While loading, the viewport holds 160 px with a slow shimmer; the frame fades in over 180 ms and its height eases over 260 ms. Reduced motion removes the shimmer and both transitions. On failure the frame is replaced by `圖表載入失敗：原因（canvas.tsx 第 N 行）` in muted text (an HTML canvas names `canvas.html`); the decision column is unaffected. When the shell refuses the canvas or cannot be asked, the same line reads `圖表載入失敗：畫布沒有回應` or `圖表載入失敗：無法確認畫布可以執行`; when the canvas replaces its own document (navigates itself to `about:blank`, a `blob:` document, or reloads) it reads `圖表載入失敗：畫布換頁了` instead of leaving a silent blank frame; when the sandbox runtime cannot be read it names that reason. In-canvas `#id` links scroll inside the frame; the frame never shows a Kunsu page. Focus never stays inside the iframe.
- **Template.** Provided by the kunsu skill (`canvas.md`, `templates/draft/`). The `kunsu` module uses the decision surface tokens (card `#0b0c0e`, line `#1d2026`, text `#f2f3f5 / #c3c7ce / #8b919b / #5f6570`, accent `#3987e5`, good `#83c494`, bad `#ef8d86`), Geist, and exposes:
  - `Stat` (label, value, unit, delta with ▲/▼, rounded percent and a 好/差 text cue, sparkline, note), `Panel` (title, note), `Grid` (auto-fit columns), `ChartFrame`, `Delta`, `Sparkline`, `fmt`.
  - A Recharts theme `chart`: 2 px monotone lines without dots, 1 px horizontal grid `#1f2227`, 12 px axis text `#8b919b` without axis lines, a y axis that sizes to its numbers with 1-2-5 ticks (萬 above 10,000), rounded bars, and a dark tooltip. Series `#3987e5 #d95926 #199e70 #c98500` (validated against `#0b0c0e`).
  - Tailwind colours `k-*` mirror the same tokens.
- **Markdown body.** Renders under the canvas with the same sanitizer as before. The link confirmation hint (`再按一次用瀏覽器開啟 網域`) only appears on links Kunsu can open (http(s)); a link whose href the sanitizer removed shows no hint and does nothing. Consecutive images form a gallery; when images are the only evidence, the gallery enlarges to two columns.

### Routines view

A `督導` section in the application sidebar. Acceptance: `docs/bdd/routine-check-in.md`.

- **Routine list.** Rows, not cards. Each row shows the source mark, routine name, last check-in, declared cadence, latest status chip (`待回應`, `已回應`, `未回應`, `退回`, `已取消`), and 30-day answered/missed/rejected counts that wrap on narrow windows. A muted inline warning `排程可能沒在跑：已 N 小時沒報到，請到 Claude／Codex 檢查排程設定` appears when the routine is overdue by twice its cadence. Only scheduled check-ins appear; Kunsu's own questions (doctor smoke, Kongko activation) do not.
- **Run history.** Selecting a routine shows its history newest first. Each entry shows time, `question_line`, the user's full answer, the AI feedback (or `AI 尚未回報`), and the handoff status. Missed, rejected and cancelled runs stay visible with their status; a run the user rejected with `這題不該問我` reads `你退回了這題：<reason>`. A default chosen by Kunsu on expiry reads `沒有回答（逾時依預設：<option>）`, never as the user's answer. Long text expands in place and wraps (including unbroken URLs); the page never scrolls horizontally.
- **No reply box.** `打開原對話` is the only path to continue the discussion.
- **Accessibility.** The view is keyboard navigable, and every status chip carries text, not colour alone. Rows are buttons inside a list; the source mark has an accessible name (Claude／Codex). Entering a history moves focus to its heading; going back returns focus to the row that was opened.

### Batch review (implemented 2026-10-09, pending QA)

A batch question (`kind: batch`) holds 1–20 items of one kind — email reply drafts, comment replies, or approve-only items such as refunds. Reference: `docs/prototypes/batch-review.html`; acceptance: `docs/bdd/batch-review.md`.

- **Layout.** The evidence column shows the selected item's original (the incoming email, the comment thread, the request). The decision column is a list of item cards: number, title (wraps; never truncated), a status chip with text (`採用`, `你改過`, `不處理`, `請 AI 改`), a one-line excerpt, an auto-growing draft field (editable items only; re-measured whenever its width changes, so the whole text is always visible), and three pill buttons. Approve-only items have no draft field but the same three buttons (`照建議`, `不處理`, `請 AI 改`). Two bulk buttons sit above the list (`全部採用 AI 草稿`, `全部退回，請 AI 重寫…` which opens one instruction field); a summary line and `送出決定` sit in the footer.
- **Defaults.** Every item starts as `採用`; items the AI recommends not handling start as `不處理` with the reason above the draft (`AI 建議不處理：…`). Their draft is empty or a real reply — never placeholder text such as `（不回）` (approved 2026-10-09). Edited items can be reverted to the AI version. `請 AI 改` requires a one-line note; until then the send button is disabled and the footer says why. An editable item can only be approved with text: while an approved item is empty the send button is disabled and the footer says `第 N 項沒有內容，先寫好再採用`. The redo-all instruction is at most 2,000 characters; over the limit its send button is disabled and the field says `整體指示超過 2,000 字`.
- **Re-registration while open.** When the agent re-registers the draft while the panel is open, items whose AI draft did not change keep the user's edits and decisions. Items whose AI draft changed show `AI 更新了這一項`: The user's edited text is kept and `還原 AI 版本` restores the new AI version; an item the user had not edited takes the new version.
- **Keyboard.** J/K or ↑/↓ move between items, E enters the draft, Esc leaves a text field (anywhere else — including the buttons inside the draft, redo-all and reject areas — Esc is `稍後再問`), A/X/R decide, ⌘Enter sends (inside the redo-all field it sends the redo-all). A plain Enter never sends; in the receipt, Enter on a link or another button only activates that control.
- **Summary wording.** `採用 N 項 · 其中 M 項你改過 · 不處理 K · 請 AI 改 L`; categories with zero are omitted, and an approve-only batch says `照建議 N 項`. Kunsu does not know whether an item is an email or a comment, so it never says `寄出／封`.
- **Narrow panels (< 1100 px, the same container query as the decision surface).** The panel becomes one column and scrolls as a whole; the send row stays pinned to the bottom, and the selected card shows its original inline because the evidence column is above the list.
- **Receipt.** The question renders as Markdown, as while deciding. Focus moves to `下一題`／`完成`. Lists every item with its status chip (and the revise note when it differs from the overall instruction), then what the AI does next: send N verbatim, the revise items come back as the next round, the skipped items are archived.
- **No money.** Batch items never carry money operations.

## Application (dub.co language, Target 2026-10-09)

Ray asked for the application window to drop its dated look and adopt dub.co's design language, with rounded corners everywhere. We take the language (layout, palette discipline, components, density), never dub's name, logo, illustrations or copy. Only the application window changes; the edge notch, overlay and decision surface keep the black system above. Acceptance: `docs/bdd/application-shell.md`, `docs/bdd/settings.md`.

**Window.** The title text is hidden and the content runs under the title bar (`titleBarStyle: Overlay`); the traffic lights sit over the sidebar, whose top 44 px is a drag region. Minimum size 900×600.

**Layout.** A 232 px sidebar sits directly on the window background. The main content lives in one white panel inset 8 px from the top, right and bottom edges, with a 16 px radius, a hairline border and a whisper shadow (`0 1px 2px rgb(0 0 0 / .04)`). The panel scrolls on its own; the sidebar never scrolls with it. The panel always reserves its scrollbar gutter (`scrollbar-gutter: stable`), so a page that starts or stops scrolling never shifts the header or the content sideways. On wide windows the page content is centred in the panel, and the header's inner row uses the same maximum width, so the title lines up with the content's left edge and the last header button with its right edge — nothing hugs the far right of a 1680 px window (revised 2026-10-09, application QA round 1).

**Tokens (light, default).**
- Window background `#F5F5F5`; sidebar hover `#EBEBEB`; panel `#FFFFFF`; subtle fill `#FAFAFA`; hover fill `#F5F5F5`.
- Text strong `#171717`, body `#404040`, muted `#6B6B6B`, faint `#A3A3A3`. (Muted was `#737373`; it measures 4.35:1 on the `#F5F5F5` window background and on hover fills, below AA, so it is darkened one step — 4.89:1 there, 5.3:1 on white. Revised 2026-10-09 during implementation.) Faint is for icons, placeholders, separators and disabled controls only, never for readable text (2.5:1 on white; the dark theme's faint `#737373` is 4:1 on its panel).
- Border `#E5E5E5`, strong border `#D4D4D4`. Switch off track `#8F8F8F` (3.2:1 on white; the strong border was 1.48:1, below WCAG 1.4.11).
- Primary button `#171717` → hover `#262626`, text `#FFFFFF`. Accent (active nav, focus, links, selected tab) blue `#2563EB`, active nav fill `#EFF4FF`, count badge on active `#2563EB` with white text, otherwise `#E5E5E5` with `#525252` text.
- Status pills (fill / text): success `#ECFDF3` / `#067647`; waiting `#FFFAEB` / `#B54708`; danger `#FEF3F2` / `#B42318`; neutral `#EBEBEB` / `#525252`; info `#EFF4FF` / `#1D4ED8`. Every pill also carries an icon (or, for the neutral `已改過` marker, a dot) and a word. The neutral fill is one step darker than the row hover fill, so a neutral pill keeps its shape on a hovered row (it was `#F5F5F5`, identical to the hover).

**Tokens (dark).** Window `#0A0A0A`; sidebar hover `#1A1A1A`; panel `#111111`; subtle fill `#161616`; hover fill `#1C1C1C`; text strong `#FAFAFA`, body `#D4D4D4`, muted `#A3A3A3`, faint `#737373`; border `#262626`, strong border `#333333`; switch off track `#6B6B6B` (3.5:1 on the panel); primary button `#FAFAFA` with `#0A0A0A` text; accent `#60A5FA`, active nav fill `rgb(96 165 250 / .12)`; pills use the same hues at 12% fill with lighter text, and the neutral pill is `#2A2A2A` (distinct from the `#1C1C1C` hover). The charcoal K icon sits on a hairline ring (`rgb(255 255 255 / .16)`) wherever it appears in the dark theme (sidebar, about, onboarding), so its rounded square stays visible on the near-black window; the artwork itself is never filtered or recoloured.

**Theme.** `外觀` offers 跟隨系統／淺色／深色; the default is 淺色. The choice is stored per Mac (`kunsu:theme:v1`) and applied to `<html data-theme>` in every application window before first paint.

**Type.** Geist Sans with PingFang TC. Page title 18 px/600; section and card titles 14 px/600; body and controls 13 px; helper text 12–13 px muted; table header 12 px/500 muted; numbers tabular.

**Radii.** Controls (buttons, inputs, selects, segmented controls) 8 px; cards, tables, popovers, code blocks 12 px; the content panel 16 px; pills, switches and count badges full. Nothing square.

**Sidebar.** Brand row (the 鎮定的 K icon at 24 px + `Kunsu`) under the traffic lights. Nav items are 32 px tall, 8 px radius, 16 px icon + label + optional count badge; hover `#EBEBEB`; active uses the accent fill and accent text.

**Selected states (one language).** Every navigation selection — the sidebar, the settings section nav and its narrow pill row — is the accent fill with accent text (the narrow pills add a hairline in the accent colour); hover is a neutral grey fill and never looks selected. Choosing one value among a few — the history filter pills and the segmented control — is the primary fill (black in light, white in dark) with the primary text, far above 3:1 against its track (revised 2026-10-09; the settings nav used a grey fill, the narrow pills black, and the segmented chip was 1.04:1 against its track). Group labels (`收件匣`, `管理`) are 11 px muted. A presence line at the bottom (`2 題正在等你` with a dot, or `目前沒有待決`) sits above `設定`.

**Page header.** Inside the panel: exactly 56 px tall at every width, title on the left (a sub-page puts its `‹` back button inline before the title), actions on the right, hairline bottom border. The page's description is not in the header: it opens the content as a muted 13 px intro line (`text-wrap: pretty`, so a wrapped line does not end on a lone character), which keeps the header one line and the narrow header free of orphaned words. Content padding 24 px; max content width 1080 px (settings: 180 px nav + 32 px gap + 760 px cards; routine history 760 px), centred; the header's inner row uses the same maximum width. The header is also a window drag region (its buttons stay clickable). The application toolbar's one `勿擾模式` control (see "Do Not Disturb") sits at the right end of every page header, after the page's own actions.

**Buttons.** 32 px tall (36 px for the page's primary action), 8 px radius, 13 px/500 text, 12–14 px horizontal padding. Primary: black fill. Secondary: panel fill, hairline border, hover subtle fill. Ghost: no border, hover fill. Danger: `#B42318` text on danger pill fill. Disabled: 50% opacity, no hover. Focus: 2 px accent ring with 2 px offset.

**Inputs.** 32 px tall, 8 px radius, hairline border, panel fill; focus border accent plus a 3 px accent ring at 15%. Time inputs use a 24-hour `HH:MM` field; number inputs show their unit as a suffix (`分鐘`, `小時`, `天`, `題`, `%`). Invalid input shows a red border and a 12 px message under the field; the save button stays disabled.

**Switches and segmented controls.** Switch 36×20, full radius, accent when on and the switch-off token when off (≥3:1 against the card in both themes), with `role="switch"`. Segmented control: a subtle-fill track with 8 px radius holding equal buttons; the selected one is a primary-filled chip (see "Selected states").

**Cards.** 12 px radius, hairline border, panel fill. Settings cards follow the dub pattern: a 20 px body (title, one-line description, controls) and a footer bar (subtle fill, hairline top) holding a muted help line on the left and `儲存` on the right; `儲存` is disabled until the card has a valid change. A card whose value differs from the default shows `回復預設` (ghost) next to `儲存`. A card whose keys are in the personal file shows a neutral `已改過` pill with a small dot — no sparkle or other AI imagery (the same goes for `重新執行導覽`, which uses a compass). Keyboard focus never falls to the page: while saving, `儲存` is `aria-disabled` (still focusable); after a save or a reset the focus moves to the card title, and whenever a control is replaced by another (do-not-disturb presets ↔ `提前結束`, the notch's `回到預設位置` ↔ `確定移回`) focus moves to the new control.

**Stats.** One bordered card split into equal cells by hairlines; each cell has a 12 px muted label and a 24 px/600 tabular number.

**Tables and rows.** A bordered 12 px card; header row on subtle fill; rows exactly 56 px with or without a subtitle (only a row carrying a two-line title or a routine's overdue warning grows), hairline separators, hover fill, whole row clickable, a chevron at the end. Columns collapse to a stacked row below 760 px panel width. Long titles truncate with an ellipsis and expose the full text to assistive technology.

**Tabs and filters.** History uses a row of filter pills (full radius, 28 px): the selected one is black (dark theme: white) with counts.

**History rows.** Clicking a history row opens the decision surface in review mode (decision-surface scene 17): read-only, with the real outcome. The history page stays underneath (the inbox never flashes in between) and closing the review returns to the same page, filter and scroll position, with focus back on the row that was opened. Every flow started from the main window returns to the page it was started from.

**Empty states.** Centered in the table area: a 40 px rounded-square tile with a 20 px icon, a 14 px/600 title, a 13 px muted line, and an optional secondary button.

**Code and commands.** Monospace 12 px in a subtle-fill 12 px-radius block with a copy button that turns into `已複製` for 1.5 s. An inline command inside prose breaks only between its flags: each flag and its placeholder (`--routine <id>`) is kept on one line.

**Settings layout.** Inside the panel a 180 px section nav (`一般`, `睡眠時間`, `強制詢問`, `到期與配額`, `勿擾`, `瀏海`, `Agent 連線`, `資料與帳本`, `關於`) on the left and the cards on the right; below 760 px panel width the nav becomes a horizontal scrolling pill row. Each section has a heading (16 px/600) and a muted description. The do-not-disturb card always states its current status first (`目前沒有開勿擾`, or the active `勿擾至 18:30 · 剩 2 小時 14 分`). Errors from a broken personal file name settings the way the page does (`強制詢問 › 週期長度`), never by their file key (`force.cycle_min`); the raw text stays in the banner's `title` for debugging. The agent connection fixes are commands the user can paste as they are: Codex installs the local plugin (`python3 ~/Desktop/kunsu/plugins/kunsu/scripts/manage.py install`), and the Claude row names what is actually checked (`Claude Code（~/.claude.json）`).

**Toasts.** Saving shows a small toast at the panel's bottom right (`已儲存，立即生效`), 12 px radius, auto-hides after 2.5 s, `role="status"`. Errors stay inline on the card.

**Motion.** Hover and press transitions 120 ms ease-out; theme switch is instant; reduced motion removes transitions. No page-level animation.

**Accessibility.** Every control has a visible label; switches and segmented controls are keyboard operable (Space/Enter, arrow keys); focus is always visible; status is never colour alone; contrast meets WCAG AA in both themes.

## Motion

Motion follows Apple-style continuity: the object that starts an action appears to become the object that finishes it.

- Micro feedback: 120–180 ms, `cubic-bezier(.2,.8,.2,1)`.
- Surface transition: 260–420 ms, `cubic-bezier(.16,1,.3,1)`.
- Capsule idle: the rim and point-orb remain calm and unchanged under hover. Drag and click are the only capsule interactions.
- Capsule release: project to the nearest screen edge while preserving its position along that edge. Within 64 logical px of an endpoint it magnetizes to the corner. Animate the shortest translation over 320 ms with the surface curve; reduced motion docks immediately. **Target:** superseded by the Edge notch perimeter spring and corner L (see "Edge notch").
- Capsule to overlay: each click starts a fresh native transition event carrying the origin's screen-space position. The black shape extends along the edge, then spreads into the panel (see "Decision surface"); the source mark flies to the header. Do not play the transition while the overlay window is hidden; the overlay is ordered front only after its ready handshake (see "Overlay presentation reliability").
- Question change: cross-fade only the panel content — the old content fades out over 140 ms, the new content fades in over 140 ms. The opening shape is not replayed, and navigation never zooms the whole screen. With reduced motion the content swaps at once.
- Press: scale to 0.975 for 90–120 ms.
- Respect `prefers-reduced-motion`; retain opacity/state changes without spatial choreography.

## Agent-generated content contract

`question_line`, `why_md`, `body_md`, evidence text, and option labels/details may contain Markdown plus safe inline/block HTML. Rendering must sanitize output. Emphasis works next to CJK full-width punctuation (`**要你決定：**用一句話` is bold), and lists show their bullets and numbers. The composer accepts Markdown as plain source text but does not render a duplicate live preview.

**Evidence canvas (Claude Artifact model).** A draft may include one `canvas.tsx`/`canvas.jsx`/`canvas.html`. It may import `react`, `recharts`, `lucide-react`, and `kunsu` (tokens, `Stat`, `Panel`, chart theme), and use Tailwind classes. It renders in a `sandbox="allow-scripts"` iframe whose CSP blocks all network, forms and popups, and the shell keeps the frame (and any frame it opens) on `about:srcdoc`, `about:blank` or `blob:`, so it can never navigate to an outside address or to Kunsu's own pages; the only messages Kunsu accepts are `resize`, `ready`, and `error`. It is framed and labelled `AI 提供的資料`. Failure or a missing `ready` within 2 s hides the canvas and shows the reason; the decision stays fully operable. Acceptance: `docs/bdd/evidence-canvas.md`.

`visual_symbol` (maximum four Unicode characters) is still accepted for compatibility but is no longer rendered: the decision surface always shows the source agent mark. It can never inject an image URL, CSS, script, or interaction.

Allowed content: headings, paragraphs, emphasis, lists, blockquotes, links, code/pre, tables, details/summary, images with safe URLs, `kbd`, `mark`, `small`, `sup`, `sub`, `hr`, `br`.

Disallowed content: scripts, styles, forms, buttons, inputs, iframes, embedded objects, event handlers, arbitrary classes, inline positioning, and external executable content.

Agents control content hierarchy only. Kunsu controls layout, font, palette, spacing, interaction, and animation.

## Decisions log

| Date | Decision | Rationale |
|---|---|---|
| 2026-08-02 | Adopt quiet command center direction | Matches Kunsu's serious interruption role without amplifying “AI” decoration. |
| 2026-08-02 | Hide capsule when there are no actionable questions | Removes persistent visual obstruction while preserving menu-bar retrieval. |
| 2026-08-02 | Allow sanitized Markdown and HTML | Gives agents enough structure to explain context without allowing generated UI to escape the product grammar. |
| 2026-08-02 | Use point-orb plus specular rim as the only attention motif | Creates product identity and consistent state feedback across all three surfaces. |
| 2026-08-03 | Replace the overlay modal with a split, borderless answer workspace | Keeps attention on one decision while removing generic card chrome. |
| 2026-08-03 | Keep answered questions in the active flow session | Users may navigate back to review an answer without mutating the append-only decision. |
| 2026-08-04 | Remove every capsule hover surface and replay handoff only on open | Hover chrome interfered with dragging; a click-scoped native event makes the orb-to-agent transition visible and spatially continuous. |
| 2026-08-04 | Remove the composer Markdown preview | The duplicated answer consumed vertical space without improving the send decision; Markdown remains accepted as source text. |
| 2026-08-05 | Make the agent image the visible 15-minute decision clock | The product exists to obtain blocked human decisions; a receding ring and three-minute warning make the force cycle legible without adding a second visual motif. |
| 2026-08-12 | Add literal Do Not Disturb for up to 24 hours | A bounded, visible pause is safer than quitting the app and preserves every unanswered decision. |
| 2026-10-08 | Frame Kunsu as the user's strictest parent | General AI products cannot compel the user; Kunsu exists to compel a response while never answering, authorizing, or trapping the user. |
| 2026-10-08 | Replace the floating capsule with the bezel-black Edge notch (Target) | A reminder that looks like part of the display is persistent without feeling like clutter. A perimeter coordinate makes edge sliding and corner L-shapes continuous. The user approved the interactive prototype. |
| 2026-10-08 | Allow hover to extend the notch along the edge and play a marquee (Target) | Reverses the 2026-08-04 no-hover rule, which existed to protect dragging. The extension grows away from the pointer, and hover is cancelled the moment a drag starts. |
| 2026-10-08 | Draw the notch with SVG in a full-display stage window with native hit-testing (Target) | L-shapes and inverse fillets need arbitrary outlines and per-glyph text layout. Keeping motion inside one window avoids janky per-frame window resizes. |
| 2026-10-08 | Use official vector agent marks (Target) | The `✳` glyph did not read as Claude; recognisability is the whole job of the mark. |
| 2026-10-08 | Overlay appears only when operable; Escape has a native fallback (Target) | A blank, unescapable full-screen surface breaks trust and violates "strict, never stuck". |
| 2026-10-08 | Evidence | Decision layout with fixed bottom-right submit (Target) | The user skims text and needs charts and context. The old layout put "這題不該問我" where users expect confirm. |
| 2026-10-08 | Adopt the Claude Artifact model for evidence (Target) | A custom chart grammar was neither flexible nor cheap to learn. Agents already write React and Recharts fluently; a sandbox plus native controls keeps the safety the bounded grammar protected. |
| 2026-10-09 | Edge notch QA round 1: hit region equals the drawn shape; distance-based polling; native enter/leave; hover starts private; head end anchors text-length changes; settle ω 20 | Visible black that let clicks through and invisible slop that ate them both broke "never in the way". A cached privacy flag leaked question text for a frame. The analytic spring needs ω 20 to settle in 380 ms. |
| 2026-10-09 | Edge notch QA round 2: save the head end, not the centre; display identity is the CGDirectDisplayID; one spring for centre and length; corner rest length adds the arc loss; Latin words include in-word symbols; marquee strips only real tags; no `時間到了` while the question is being asked; native press latch; standard menu bar height floor | Every one of these was a way for the mark to drift, the last glyph to fade, a number to fall apart, content to vanish, or a click to fall through. The head end is what the user's eye anchors on, so it is what persists and what never moves. |
| 2026-10-09 | Edge notch QA round 4: grab on the arm under the cursor; tabular digits keep the countdown length fixed; a hidden stage ends the web drag; an undrawn release saves the drawn head; no rest-text fade on a privacy change without hover; a new display identity tells the page even at the same size | A press near the corner of an L jumped the head 30–40 px once the drag changed edge. A countdown that resized the notch every second broke FR-03 (no layout shift). A drag left in the page after Do Not Disturb hid the stage could recapture the pointer. Main line decided these on the user's behalf on 2026-10-09. |
| 2026-10-09 | Edge notch QA round 5: a press in the corner square of an L picks its arm by the first 4 px of movement; Settings' reset is its own event and drops a pending save; countdown minutes are padded to the warning length's digits | The square is on both arms, so the nearer edge was a guess, and a wrong guess froze the head for up to 40 px. A missing saved value on re-read also meant "the debounced write has not landed yet", so the notch jumped back to the default and the late write undid a reset. A warning of 10 minutes or more lost a digit at `9:59` and changed the notch length. Main line decided these on the user's behalf on 2026-10-09. |
| 2026-10-09 | Edge notch QA round 3: hover always grows from the tail and turns a corner into an L; the notch rests wholly on its edge (longer text shifts it back to the 24 px release gap rather than wrapping); release snaps with the final length; hover collapses anchored at the head, also when a drag starts from hover; reduced motion snaps every target; the overdue text waits for the first reason; an undrawn notch clears its drag on both sides | The mark is the anchor the user's eye returns to, so it never moves for hover. An off-centre L at rest, a 28 px gap and a flashed `馬上跳出來` each contradicted the spec. A drag left over from a vanished notch could capture another app's drag. Main line decided these on the user's behalf on 2026-10-09. |
| 2026-10-09 | Batch summary says `採用 N 項`, and a narrow batch panel shows the original inside the selected card | Items carry no medium (mail, comment, refund), so `寄出／封` would be wrong half the time. In one column the evidence sits above the list, so the original has to move to where the user is looking. The user approved both on 2026-10-09, along with panel-only answering for batches. |
| 2026-10-09 | Decision surface QA round 3 and batch QA round 2: hiding never runs ahead of an answer in flight and the panel stays mounted through the hide grace; the main window follows the live list; option links sit outside the radio button; history review is read-only with the real outcome; one dismissal per question per overlay session; the `稍後再問` error gets its own row | The last receipt vanished because the list emptied before the answer returned. A link inside a radio button could neither open nor be clicked without toggling the option. An expired question offered an answer box the server would refuse, and an agent's cancellation read as the user's rejection. A retry after a partial failure dismissed the current question twice. The user approved these fixes on 2026-10-09. |
| 2026-10-09 | Decision surface QA round 2 and batch QA round 1: one breakpoint measured on the panel (< 1100 px) for both; questions forced mid-session join the session; the last receipt is held only while it is showing; stepping aside finishes before a link opens; a failed `稍後再問` stays open with an error; batch drafts never carry placeholder text | Two breakpoints that disagreed by one pixel left a 1100 px batch unscrollable. A question that became due behind an open overlay never appeared. A placeholder such as `（不回）` would be sent verbatim once the user switched the item to `採用`. The user approved these fixes on 2026-10-09. |
| 2026-10-09 | Application window adopts the dub.co design language (light default, dark variant, rounded everywhere, settings cards with save bars); settings expose sleep time, force cycle, expiry and quota, fatigue, appearance, do-not-disturb, notch reset, real agent status, ledger health and about; personal values live in `~/.kunsu/policy.local.toml` | Ray found the settings window dated and asked to copy dub.co with rounded corners and complete settings. The install script overwrites `~/.kunsu/policy.toml`, so the user's own values need a file it never touches. Forcing surfaces keep the black system because they must stay unmistakable. |
| 2026-10-09 | Money receipt follow-up follows the server's `money_state`; confirm rows own focus; the money row icon carries the state | The 8/6 dead-lettered question was unknown on the server but the receipt read the text and offered nothing, so the user could never clear it. Buttons frozen at mount, focus dropped to `<body>` after a confirm, and a ✓ on an unknown outcome each broke the receipt's promise. Main line decided these on the user's behalf on 2026-10-09 (money QA round 11). |
| 2026-10-09 | Application implementation: light muted text `#6B6B6B` (not `#737373`), faint never carries text, the 勿擾 control lives in every page header | `#737373` fails AA on the window background the sidebar sits on. The page header is the only toolbar the dub layout has. |
| 2026-10-09 | Application QA round 1: centred content with an aligned header row, a stable scrollbar gutter, a one-line 56 px header (the description opens the content), 56 px rows, one selected-state language (accent for navigation, primary for choices), a ≥3:1 switch-off track, a neutral `已改過` dot pill, focus that never falls to the page, review that returns to its page and filter, a dark-only ring on the K icon, a neutral pill distinct from the hover fill; the decision surface declares dark itself, sits on black in the main window, names the real cycle length in `N 分鐘後再問`, offers `關閉` instead of `稍後再問` in review, and keeps full-width punctuation in PingFang | Each was a place where the window looked unfinished or broke a WCAG rule: a 1680 px window left the content stranded on the left, a header changed height with its description, rows changed height with a subtitle, a switch disappeared when off, three different "selected" looks, a sparkle read as AI magic, focus fell to the page after saving, and a review dropped the user back on the inbox. Main line decided these on the user's behalf on 2026-10-09. |

## Assistant reply inbox (Current, 2026-10-10)

Editable batches use a compact source/count strip (course, Facebook, Instagram, Threads, LINE, other) with monochrome Lucide icons and text labels. Category chips navigate, never hide sendable drafts. Left rail contains the selected source and original; right rail prioritizes full editable drafts. Green “確認／已確認” is only a local review bookmark, toggles without submitting, and resets on edits/source changes. The bottom primary action says “全部送出 N 則” with explicit pending/rewrite/skip totals; it submits current visible text without requiring bookmarks. Empty text and missing rewrite instructions remain blocked. Draft text/actions/bookmarks survive closing and reopening on the same device; errors preserving changes are visible. Existing keyboard, reduced-motion, long-text and sanitizer guards remain in force.

### Queued Codex handoff (2026-10-10)
A queued continuation says 「已排入原對話，等待 AI 接手」 in muted text. It must not display a completed checkmark before an AI report. The existing report pane remains the actual completion receipt.

## Mobile app (Target, 2026-10-10; prototype stage)

PRD §30. Prototype: `mobile/` (SwiftUI, mock data only). Interaction-model exploration (2026-10-12, Ray found the mixed card + chat stream unintuitive): [`docs/prototypes/mobile-concepts.html`](docs/prototypes/mobile-concepts.html) compares four models — messages app, one item at a time, a one-question-at-a-time chat with 軍師, and split 決定／對話 tabs. The user preferred the one-question-at-a-time chat (2026-10-12) with two constraints: every open starts clean (no full conversation, no visible history) and history lives somewhere reachable. Five variants of it: [`docs/prototypes/mobile-kunsu-chat.html`](docs/prototypes/mobile-kunsu-chat.html) (clean chat + history button, collapse answered items, pull up for earlier, briefing first, ask-back for context). Current candidate (2026-10-12): [`docs/prototypes/mobile-kunsu-chat-merged.html`](docs/prototypes/mobile-kunsu-chat-merged.html) — opening shows only this visit — history is not rendered until a deliberate pull-down at the top: a ring fills with the pull distance (`向下拖出歷史紀錄` → `放開看歷史紀錄`; the user renamed it 2026-10-12), release when full loads one more day with a short reveal, release early springs back; every bubble's avatar is the AI that asked (軍師's own lines use the K mark), and the status plus project is one inline tag at the start of the question (`要你決定 · kunsu`), never an extra row; under the live question, a plain row (no bubble, no avatar) overlaps the next items' source marks followed by `尚有 N 個待辦事項要處理`, tap to expand and pick one; answers are colour-and-shape tiles (▲◆●■, 2×2 when every label is short, one per row otherwise, option details in the question bubble keyed by the same shape); an answered item merges instead of shrinking (question bubble unchanged, your answer carries a status line, ask-backs fold into `追問 N 次 ▾`); `為什麼？`／`前因後果？` stay above the tiles. The Stream and Home sections below stay as the SwiftUI baseline until the user picks one. The job is **the shortest path from a lit phone to an agent working again**, usually in bed, at night, one-handed. Every screen is judged by taps-to-unblock and by never answering the wrong thing.

**Look.** Light and dark themes, following the system by default with a manual override (Ray, 2026-10-12; web prototypes ship both too). Dark is the black system of the decision surface: canvas `#000000`, card `#0B0C0E`, raised `#141519`, line `#1D2026`, text strong `#F2F3F5`, body `#C3C7CE`, muted `#8B919B`, faint `#5F6570` (icons and separators only). Waiting `#D9B26A`, danger `#EF8D86`, complete `#83C494`, accent `#3987E5` (links, focus, selected). Source marks are the only other colour (Claude `#D97757`, Codex lavender→indigo). Light: canvas `#FFFFFF`, card `#F6F7F9`, raised `#ECEEF2`, line `#E1E4E9`, text strong `#111318`, body `#3A3E46`, muted `#646973`, faint `#A3A8B0`, waiting `#8A5A00`, danger `#B42318`, complete `#1F7A3A`, accent `#2563EB`; the primary pill inverts (near-black fill, white text). No gradients, no glass on lists.

**Type.** System SF Pro with PingFang TC, and SF Mono for code and numbers (deviation from Geist: native Dynamic Type and Chinese metrics matter more on iPhone). Every size follows Dynamic Type; the base mapping at default size: screen title 28/700, section label 13/600 muted, row title 17/600, body 16/400 at 1.45 line height, question 24/700, meta 13/400 muted.

**Geometry.** 4 pt grid; 16 pt side gutter; rows ≥ 64 pt; tap targets ≥ 44 pt; option cards ≥ 56 pt; primary button 52 pt tall, full radius, white fill with black text. Radii: chips full, cards 16, sheets 28.

**Navigation.** One stack, no tab bar. Home → Conversation. `新對話` is a floating 56 pt circular button bottom-right on Home. Settings sits behind the K mark top-left. The decision card opens as a full-height sheet over whatever is on screen.

**Home (`等你`).**
- Header: the K mark, title `等你`, and a connection line — `我的 MacBook · 已連線` with a complete dot, or `Mac 離線 · 最後同步 23:10` with a waiting dot plus text. Offline never renders as an empty state.
- Sections in fixed order, empty sections hidden: `要你決定` (Kunsu questions), `等你回話` (sessions whose last turn asks the user something or waits for a permission), `執行中`, `最近`.
- A decision row shows the source mark, the question line (2 lines max), the asked time, and — for a plain single-choice, non-money question — up to three option chips that answer in place with the undo bar. Anything else shows `打開` and goes to the sheet.
- A conversation row shows source mark, project name, the last AI line (2 lines), time, and a status word with icon (`等你回話`, `等你批准`, `執行中 · 跑測試`, `閒置`). `等你回話` rows carry a `繼續` chip that sends "繼續" with the undo bar.
- When everything is clear and the Mac is online: a quiet centred line `沒有事在等你` and nothing else.

**Conversation.**
- Header: back, source mark, project name, branch in mono, and a status word. If the desktop is running this conversation right now: a banner `桌面上正在跑，你的訊息會排隊等它空下來`.
- Transcript: the user's messages are right-aligned raised cards; AI text is plain left-aligned body text without a bubble. Tool calls collapse to one muted row with an icon and a summary (`改了 3 個檔 · 跑了測試 ✓`); tapping expands the list. Code blocks fold after 8 lines.
- Permission request: an inline card with the command in mono, the reason, and `允許一次` (primary) / `拒絕`. No Face ID anywhere in the flow (Ray, 2026-10-12).
- Pinned decision: when this conversation has a pending Kunsu question, a compact black card sits above the composer (`要你決定` + question line + `回答`). It opens the decision sheet; the composer can never answer it.
- Composer: while a permission request is pending, no quick-reply chips are shown (a `繼續` there would read as an approval). Otherwise, a row of quick-reply chips (`繼續`, then up to three options extracted from the last AI message, e.g. `A`, `B`, `照你建議`), a growing text field, a 44 pt mic button, and send. Sending shows the undo bar.

**Stream mode (A/B with Home, 2026-10-10).** One conversation between the user and 軍師 replaces Home; the K mark's menu switches `對話流`／`清單` (stream is the prototype default).
- **Order.** Grows bottom-up like a chat. Everything already handled sits above a divider as history; everything still waiting sits below the divider `還在等你 · N`, closest to the composer. Within the waiting group, urgency increases downward: running lines (`ads-review 執行中 · 跑報表`, one muted row each), then AI messages waiting for a reply, then permission cards, then Kunsu decisions at the very bottom.
- **Items.** Every item from a session carries a header (source mark, project, time); tapping the header opens that full conversation (the existing Conversation screen). AI messages render as left-aligned cards without a bubble tail. The user's replies are right-aligned raised cards with a small `→ kunsu` line naming where they went. Decisions and permissions are the same cards as elsewhere and are answered only with their buttons.
- **Reply target.** A chip above the composer always reads `回覆給 ‹mark› kunsu · Codex ▾`. Under the chip, a quote bar (3 pt muted rule + two lines of the target's last AI message) keeps the target readable even when its card has scrolled away behind the decisions. The default is the bottom-most item that accepts text (a conversation waiting for a reply); a decision never becomes the text target. Tapping any conversation item makes it the target and gives that card a white ring plus a `回覆對象` label. The chip's menu lists the waiting conversations first, then the others, then `新對話 · Claude`／`新對話 · Codex` with recent projects; a new-conversation target reads `新對話 · Claude · kunsu`. No AI decides the target.
- **After a send.** The user's reply joins the history together with the AI message it answered; the conversation becomes a running line; when the AI answers again, its new message appears at the bottom of the waiting group. The undo bar works as everywhere, and undo puts everything back where it was.
- **Quick replies.** The chips above the composer belong to the current target (same extraction rules, none while a permission is pending).
- **Empty.** With nothing waiting, the divider reads `沒有事在等你` and the target falls back to the last conversation used.

**Decision sheet.** The phone version of the decision surface: header with source mark, agent, `第 N 題，共 M 題`, and `稍後再問` at top right; the `要你決定` label; the question at 24/700; consequence and deadline; for money, the `所見即所簽` params block; option cards (number, label, `AI 建議` badge, detail) with a white ring when selected — nothing is preselected, not even the AI recommendation, and the primary pill stays disabled until the user picks one (same explicit-selection rule as the desktop); an optional note; a footer pinned to the bottom with `這題不該問我` (text) and the primary pill (`送出決定`, or `批准並執行` for money after the 所見即所簽 confirm sheet; no Face ID). The receipt replaces the content in place, with the same handoff wording as the desktop.

**Undo bar.** Every non-money send (chip, message, decision) shows a bottom bar for 5 s: `已送出「繼續」` + a draining line + `撤回`. Nothing leaves the phone until it expires. The draining line is accompanied by the text, never colour alone; with Reduce Motion the line does not animate and the text shows `5 秒內可撤回`.

**New conversation.** A sheet: Claude | Codex segmented control (remembers the last choice), recent projects list (name + path in mono), permission mode (`Auto` default — Claude auto mode / Codex auto: routine actions run, risky ones come back to the phone as permission cards — or `會問我`; never a bypass option; approved 2026-10-10), a large text field with mic, and `開始`.

**Notifications.** The payload is decrypted on the phone. A plain single-choice question shows its options as notification actions; money, permission and multi-question items show only `打開`. Waiting conversations show the last AI line and a `繼續` action.

**Motion and feedback.** Native push/sheet transitions only. A light haptic on every send and on undo. Reduce Motion: no undo-bar animation, sheet cross-fades. No decorative motion.

**Markdown (2026-10-12).** AI-written text on the phone — the question's why/body, 軍師 answers, ask-the-agent replies, chat, task results and the last AI line of a conversation — renders a safe Markdown subset natively (no web view): paragraphs, headings (as bold lines, no bigger than the question), bullet and numbered lists with their markers, block quotes (left rule), fenced code (monospaced, scrolls sideways instead of wrapping), and inline bold, italic, strikethrough and code. Emphasis works next to CJK punctuation, as on the desktop. Links show their text only and are not tappable (no navigation from AI text on the phone); images show their alt text; raw HTML tags show as plain text. Questions, option labels and the user's own messages stay plain text. Markdown never changes the bubble width or the controls.

**Accessibility.** VoiceOver labels name the source (`Claude`／`Codex`), status words carry text and icon, every control is ≥ 44 pt, and Dynamic Type up to AX3 reflows rows to stack instead of truncating the question.

## Unified design language (2026-10-12; decision surface Current, phone app in progress)

Ray found the three surfaces drifting apart (light dub-style main window, black decision overlay, a third look on the phone) and asked for one language, defaulting to the main window ("Station") wherever unsure. Reference: [`docs/prototypes/unified-design.html`](docs/prototypes/unified-design.html) — Mac main window, decision overlay and phone app on one token set and one mock data source, light and dark. The desktop decision surface and batch review now use the Application tokens (light and dark, following the 外觀 setting in both the main window and the overlay window); where the older "Decision surface" text above says black (`#000` panel, `color-scheme: dark`), this section wins. The edge notch stays black. Known gap: the macOS overlay window cannot blur what is behind it from CSS, so the desktop panel is a near-opaque panel with hairline and shadow rather than true glass; the evidence canvas keeps its dark template on its own dark frame until it learns the theme.

- **One token set.** The Application tokens (light and dark) are the only palette for the main window, the decision overlay and the phone app. Primary action = the primary fill (near-black in light, white in dark). The only accent is the Application blue, used for selection, focus, links and the nav. Semantic red/amber/green appear only inside small status pills and error text. The Claude and Codex marks are the only other colour. No coloured answer tiles.
- **Glass means "floating above content, needs you now".** Liquid glass (translucent fill, 24 px blur with saturation, a bright inner top edge, soft shadow) is used for the decision overlay panel, the phone's answer dock and its buttons, the phone header pill, toasts and sheets. The main window's content panel and cards stay solid. Glass never carries body-length reading text without a solid card under it.
- **Exception: the edge notch stays pure black.** It imitates the display bezel; the panel that grows out of it is the glass decision overlay.
- **Decision overlay.** Glass panel (22 px radius) over a light scrim; header with source mark, `Agent · project`, `第 N 題，共 M 題` and `稍後再問 · 15 分鐘後再問 · Esc`; 24 px question; option cards on a stronger glass fill with a hairline, the selected one ringed in the strong text colour with a filled number key; `AI 建議` as a small accent pill; footer `這題不該問我` (text) and the primary pill. Money keeps the `所見即所簽` block and reads `批准並執行`. No Face ID.
- **Phone app.** Waymo-like hierarchy: a large greeting (`晚安` + `有 N 件事在等你`) opens each visit. When everything on screen fits (greeting plus this visit's items), the block sits centred in the space between the header pill and the dock, never stuck to the dock; once it grows taller than that space it scrolls normally and the newest item stays nearest the dock (2026-10-12, Ray). **Pull for history (2026-10-12, Ray: a tap opened history).** Only real overscroll past the top counts — the content's resting position never does — and history loads only when a finger drag that reached 120 pt is released; a tap, a short drag or momentum never loads it. While pulling, a 26 pt ring fades in from empty and its stroke fills in step with the distance (`向下拖出歷史紀錄`); at 120 pt the ring is full, turns solid, gives one light haptic tick and the text becomes `放開看歷史紀錄` (pulling back below the threshold reverts both, no repeat tick until it crosses again); release when full reveals one more day above with a 240 ms fade-and-slide (none with Reduce Motion); release early springs back.
- **Phone app — question and dock.** The live question is a solid card with the asker's mark as avatar and no status tag; the user's answers are primary-fill bubbles with a one-line status underneath. The answer dock is glass: `為什麼？` `前因後果？` `等下再說` as small glass chips, then glass answer buttons, then the input row. Short answers (≤ 4, each ≤ 8 CJK-width and without detail) sit two per row; any long label or detail makes them one per row with wrapping text and the detail on a second line; when the buttons exceed 38% of the screen they scroll inside the dock with a fade and `往下還有選項`, and the input row stays pinned. The AI recommendation is a 1.5 px strong ring plus a small `AI 建議` pill.
- **Type and shape.** Geist with PingFang TC everywhere; Station radii (8 controls, 12 cards, 16 panels) on desktop; the phone uses larger radii (20–22 cards and buttons, 30 dock) for touch.
- **Phone app — follow-ups and the relay chat (2026-10-12).** Follow-up answers render inside the live question card under a `你問：…` line, attributed to `軍師（Claude）` (the relay) or `<project> 本人` when forwarded; a relay answer shows `問 <project> 本人` as a quiet text button when the question has a waiting conversation. While the relay or the agent is thinking the line reads `正在想…` / `已轉問 <project>，它回答前這題會留著`. With nothing waiting, the dock drops the answer area and shows suggestion chips (`大家在做什麼？`, `開新任務`, `今天做了什麼？`) above the input, whose placeholder reads `跟軍師說…`. Relay messages are plain text beside the K mark carrying a small badge of the underlying model; a task the relay starts appears as a compact card (source mark, project, `執行中` / `等你回話`). The header pill (K mark, `軍師 · Claude`, connection) opens Settings: relay AI (Claude／Codex), appearance (跟隨系統／淺色／深色), Mac connection.
- **Mac main window — relay setting.** Settings gets a `中轉 AI` card (segmented Claude／Codex, one-line help, `儲存`) in the existing card pattern. No chat on the desktop.

