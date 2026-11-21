# M1b12: keys in the plugin page do not reach the DAW

Ticket #561, umbrella #503 (the DAW plugin program), part of M1 (#510). Follows M1b7 (#554: the
page moves the host). The plugin lives in `plugins/ara/`. Nothing in `static/` or `crates/`
changed.

## The problem

The owner pressed a key in the plugin page in a real DAW. The page did not use the key: its
command is absent in host mode (#539). The DAW took the key as one of its own key commands and ran
it on the session (a clip trim). A key press in the plugin page must never change the session of
the host. That is a risk for the data of the user.

## How a key travels on macOS

The plugin runs as a CLAP wrapped to VST3. Its editor is a `WKWebView` (wry) inside a view that the
host gives (`clap.gui`, cocoa, embedded; the VST3 wrapper passes the view of `IPlugView::attached`
to `set_parent`). The path of one key press, for a host that is a normal Cocoa app:

1. The window server gives the event to the application. `[NSApp sendEvent:]` takes it. A
   LOCAL EVENT MONITOR (`NSEvent addLocalMonitor...`) runs here first and may change the event or
   drop it.
2. A key down with Command, Control or Option may be a key equivalent. AppKit then looks for a
   menu item with that key. The test host showed that the MAIN MENU is asked BEFORE the views of
   the key window: with a menu command on Cmd+Z the command ran and the page saw no key at all.
   A view of the plugin cannot stop this step. (In the same run no view of the window took the
   key. In a separate trial a direct call of `performKeyEquivalent:` on the `WKWebView` answered NO
   6 times of 6 and gave the page no key: WebKit does not take these keys that way. The page gets
   Cmd+Z from the key down below.)
3. If no menu item takes the key, AppKit sends the key down to the FIRST RESPONDER of the key
   window. The plugin makes the web view the first responder (`makeFirstResponder`, at `show` and
   at `set_parent`).
4. WebKit sends the key to the page (a DOM `keydown`). If a handler of the page calls
   `preventDefault`, or a text field uses the key, the key is used and goes no further.
5. If the page did NOT use the key, WebKit gives the key down to the next responder: the
   superview of the web view, then the views of the host, then the window and the application.
   This is the path that the owner found: the host's own key commands live there. The test host
   showed it: with the rule off, plain keys that the page did not use arrived at the host's view as
   key down.
6. Key up goes to the first responder and then up the same chain (the test host saw it). A change of
   the modifier keys (`flagsChanged:`) and an editing command that nobody took
   (`doCommandBySelector:`, `tryToPerform:with:`) are taken by the shield in the same way; no stage
   posts them.

What the VST3 side adds: clap-wrapper 0.17.0 gives the view of the host to the plugin unchanged
(`WrappedView::attached` passes the parent to `set_parent`), so the steps above are the same.
The wrapper also has the `IPlugView` key calls `onKeyDown` and `onKeyUp`. They return
`kResultFalse` ("not handled") and do not reach the plugin. A host that gives its keys to the
plugin view through those calls (the VST3 SDK has them for hosts that do not hand native key events
to a plugin view) sees "not handled" and runs its own key command. The plugin cannot stop that
and this part does not change the wrapper. See "Open points".

## The rule

With the keyboard focus in the plugin view, EVERY key down, key up, modifier change and key
equivalent is consumed by the plugin, whether the page used it or not. This holds for keys of
commands that are absent in host mode, for keys with no command, for keys with a modifier, and
while a text field of the page has the focus. Nothing goes on to the host except the exception
list.

The role models do it this way (the ticket says so; it was not checked in their code): their
editors take the keys when they have the focus, and the space bar still starts and stops the DAW.

### The exception list

One short list, in code as `SYSTEM_KEYS` (`plugins/ara/src/keys.rs`) and here. It holds the keys
of the commands of the system, which the user expects to work in any app:

| Key | Why it goes to the host |
| --- | --- |
| Cmd+Q | quit the application |
| Cmd+H | hide the application |
| Option+Cmd+H | hide the other applications |
| Cmd+M | minimise the window |
| Cmd+` | next window of the application |
| Shift+Cmd+` | previous window of the application |

The match is exact: Shift+Cmd+Q (log out) is not on the list and stays in the plugin. Keys that
the system takes before the application (Cmd+Tab, Cmd+Space, the screenshot keys) never reach the
plugin and are not a question. Cmd+W (close) is not on the list: it is a command of the host on
its own windows and sessions, and the page has its own key for it.

The space bar is NOT on the list. It plays and stops the host through the page: the page takes the
key (its play command calls `B.hostTransport.play` or `stop` of #554), and the key does not reach
the host as a key. If the page does not use the space bar (no project, a text field), the plugin
takes it too.

Any other exception needs a reason in this table and a stage.

## How the plugin holds the rule

Three parts, all in `plugins/ara/src/keys.rs`:

1. **The key monitor** (`watch`). A local event monitor runs at step 1, before the main menu, before
   every view and before the responder chain. While the plugin's view has the focus (the first
   responder of the event's window is the web view or a view below the shield), the monitor takes
   EVERY key down, key up and modifier change of that window, gives it to the web view itself
   (`keyDown:`, `keyUp:` or `flagsChanged:`, as AppKit gives it to the first responder), and drops the
   event. A key on the exception list, and every key while the focus is elsewhere, goes on unchanged.
   The first version took only keys with Command, Control or Option (they are the keys that the menu
   sees first); a plain key still reached the host through the responder chain of an unused key, and
   Escape reached the views of the host through the window (see "Escape" below). The monitor is made
   when the editor opens and removed when it closes, on the main thread.
2. **The key shield.** The web view is a child of one plain view of the plugin, and that view is the
   child of the host's view. The shield is the next responder above the web view, so a key that the
   page did not use (step 5) arrives at the shield first. It takes key down, key up, modifier change
   and an editing command that nobody took, and does not call `super`. It is a second line: it holds
   for a host in which the monitor does not run first. The shield also takes `performKeyEquivalent:`,
   but only while the plugin's view has the focus (AppKit offers EVERY key down to the views of the window,
   whoever has the focus; the first version did not check this and the stage `editor-focus` failed). The web
   view is its own next key view: WebKit asks the window for the next key view when the user presses Tab at the
   last element of the page, and the focus must not leave the plugin there (the stage `editor-keys-tab` puts a host view that takes the focus beside the plugin and presses Tab four times at the last
   element of the page: the focus stays in the plugin. The stage has NO fault run: in the test host the focus stays in the page
   also without this guard (WebKit cycles inside the page), so the guard is not proven by the test host. It can be overwritten
   when a host recalculates its key view loop.)
   `doCommandBySelector:` (the editing command that WebKit derives from a key it did not use; measured:
   `noop:` for most keys, `complete:` for F5) is always taken while the plugin has the focus: it is only reachable from below the
   shield. WebKit finishes a key in another process, so the command can come late (a host whose main thread stalls).
   Round 2 accepted it only for an event younger than 0.25 s, counted from the creation of the event; a host view above the
   plugin that implements the command (a delete command would delete data) then got it. The stage `editor-keys-slow` shows
   the old rule leaking `noop:` and `complete:` to a host view and the new rule stopping them.
   The shield answers `tryToPerform:with:` (a menu action of the host with no target: Save, Undo, Close) only for an
   action that WebKit derives from a key of the plugin: the plugin has the focus, `take` gave a key to the web view in the
   last 2 s (counted from the handling, a monotonic stamp), the event being handled is no mouse event (a click on a menu item
   of the host has one) and no system key (the nil-target action of Command+H, Q, M must reach the host). Any other action
   goes on up the chain. The stage tests an action that comes later than 2 s after the last key; the mouse-event
   rule itself is a unit test of the decision (`action_is_ours`) only, because a posted event does not change the current event.
3. **The mouse monitor.** The keys follow the first responder, and AppKit moves it on a click only if the
   clicked view accepts it; many views of a DAW do not. A click on such a view would leave the web view as
   the first responder, and the plugin would take every key: the keys of the DAW would be dead. A mouse
   monitor takes the first responder away from the web view on a mouse down outside the plugin, and gives
   it to the web view on a mouse down in the plugin. "In the plugin" is decided by the hit test of the window's
   content view (the view the click lands on is the web view or below the shield), not by the rectangle of the
   shield: a host view over a part of the plugin (an overlay, a resize grip) is outside. On a click outside the first
   responder becomes nil (the window) and the click goes on: the host sets its own responder from the click if it wants one.
   The plugin stores no responder (a stored one could be the web view itself after a second `gui.show`, and a weak
   reference to a responder class that forbids weak references aborts the process). When the focus leaves, the page gets
   a change of the modifier keys with no modifier down, so it does not keep a Command that it saw go down, and the set of
   taken key codes is cleared. The stage calls `gui.show` twice with the focus in the page before it clicks outside.

   Key up and modifier changes across a change of focus: the plugin keeps the key codes whose key down it took. The key
   up of a key whose down went to the host (a system key, or a key pressed before the click into the plugin) is not taken: the
   monitor hands it to the responder above the shield, so the host's VIEWS get it, as they would if the web view had passed it
   on. A change of the modifier keys goes to the page (the web view) and to the same responder above the shield. Normal
   dispatch does not do this by itself: the web view keeps a modifier change for itself (measured: the host view got no
   release with the plain pass-through). A modifier alone runs no command.

The image is pinned: an Objective-C class cannot be unregistered, and a shield or a pending callback of the page holds code of
this image, so a host that unloaded the bundle would make a later call go to freed code. At the first editor the plugin opens
its own file again with `RTLD_NODELETE`, so the host's unload leaves it mapped. The cost: a new version of the bundle needs a
restart of the host (it does anyway once a class exists).

A delivery build logs no character of a key: the one row it writes has the key code and the modifier flags only (the character
may be the first letter of something private typed into a field).

The class of the shield is made at run time (`ClassBuilder`) with a name that holds the address of this
copy of the code (`DrumSpikeAraKeyShield_<address>_<n>`). A fixed name makes the second copy of the
plugin in a process (the CLAP and the VST3 bundle, two versions, a reload) panic when it registers its
class. The stage `editor-keys-twin` loads the CLAP and the CLAP of the VST3 bundle into one process, the
first still loaded, and opens an editor in each.

Clipboard keys: Cmd+C, Cmd+X and Cmd+V are menu items in an app, and a text field of the page would lose them
when the host's menu cannot get them any more. After such a key the plugin asks the page (`__DS_KEYS__.edit()`)
whether a text field has the focus and no handler of the page used the key; if so, the web view runs
`copy:`, `cut:` or `paste:`. Select All, undo and redo in a text field are run by the page script
(`execCommand`), also only when no handler used the key. (Not measured: the stages do not touch the
pasteboard of the user.) Command+C of selected read-only text of the page also copies (the page asks `getSelection()`); the
page stops `selectstart` outside text fields, so read-only text cannot normally be selected, and this is untested. Cmd+Z in a field is measured: it undoes the typing.

A held space bar repeats its key down, and each would play or stop the host. The plugin's page script drops a
repeat of the space bar before the page sees it, unless a text field has the focus (stage `editor-keys-space`).

Reports: a gate build writes a row for each key the plugin takes (the first 400). A delivery build writes at most one
row, for the first key taken, keeps it in memory (never writes the file from the host's main thread), and counts the
rest for the row `editor_close` (`keys_stopped`, `keys_passed`).

### Escape

In round 2 an unused Escape key reached the views of the host above the plugin: WebKit asked the responder chain for
`cancelOperation:`, and the answer of the shield was wrong for a late command (the 0.25 s rule). Round 2 also had a script
guard (the page marks Escape as used); round 3 removed it, because the shield now always takes the command while the plugin has
the focus (see the shield above) and the stage `editor-keys-host-record` holds with no guard. The fault run `old-freshness` shows the old behaviour.

### Layouts

The system keys are matched by character (Cmd+Q, Cmd+H, Cmd+M: `charactersIgnoringModifiers`) and by key code
(the window keys). On a layout with no Latin letters (a Cyrillic or Arabic layout without a Latin mode) the
character of Cmd+Q is not "q": the key is not recognised as a system key and the plugin takes it. That is the
safe side: a key that is not surely a system key stays in the plugin. The user then presses Cmd+Q with the
focus outside the plugin. Not tested: dead keys, the accent menu of a held key, and input methods (IME). Control+Command+Space (the character picker)
is taken by the plugin while it has the focus. Tab can move the first responder to the host when the host recalculates its
key view loop: the keys then follow the first responder, which is the host's, and there is no data risk. Option keys that type characters (the German at sign, brackets) go to the text field of
the page: the stage types Option+L, Option+5 and Option+6 into the field.

### The text for an absent command

A key that the page does not use is silent. When the key belongs to a command that is absent in
host mode, the page shows one short text: the text of the command's group from #539 (for example
"the host plays the audio" for U, "the host owns the files" for Cmd+E). `plugins/ara/src/editor/keys.js`
does it from the plugin's side, with no change in `static/`:

- It classes each `keydown` with the page's own tables (`DSKeys`, `HOST_ABSENT`, `hostOwned`): a
  command of the page, a command that host mode leaves out (with its group), or no command.
- When the key is of an absent command, no handler of the page used it (`defaultPrevented`, read when
  the dispatch ended) and it is not a letter typed into a text field, it calls the page's own
  `hostAbsent(group)`, which sets the status line.
- A key with no command at all stays silent, as it does in the app.

A change in `static/` that would make this simpler (the page showing the text itself in its key
handler) is not needed and was not made.

### Who has the keys: what the user sees

The keys follow the first responder. A click in the page makes the web view the first responder:
the plugin has the keys. A click anywhere else in the host (its own view) moves the first
responder away: the host has the keys again. The plugin draws no frame for this. The user sees it
in the page: a text field shows its caret, a key of an absent command shows its group text, and the
page's own keys work. If the page does not react to a key and shows no text, the host has the
keys. (Open point for the owner: a thin frame of the page while it has the keys.)

## The test host and the stages

The `hostkeys` mode of the test host (`ara-host ... editor --editor-mode hostkeys`) gives the host a
content view and a main menu that RECORD every key that reaches them:

- the host's content view records key down, key up and modifier changes that arrive at it through
  the responder chain (what a DAW's own view does with a key that nobody took);
- the host's main menu has key commands as a DAW has: Space, G, U, Cmd+E, Y, Cmd+J, Option+X, Cmd+Z, Q,
  and the system keys Cmd+H and Cmd+Q. A menu command records its title.

The host posts REAL key events (`NSEvent` key down and up through `[NSApp sendEvent:]`, to the window
of the test host; never a global event) and two clicks. The page driver records what the page saw
of each key, and the text of the status line. The steps, one every 900 ms:

| Step | Key | What it is |
| --- | --- | --- |
| space | Space | the space bar, page transport |
| page-key | G | a command of the page |
| absent-key | U | key of an absent command (audio group) |
| absent-key-cmd | Cmd+E | key of an absent command with a modifier (files group) |
| no-command, no-command-cmd, no-command-option | Y, Cmd+J, Option+X | keys with no command, plain and with a modifier |
| page-key-cmd | Cmd+Z | a command of the page with a modifier |
| field-letter, field-cmd-absent, field-cmd-none | U, Cmd+E, Cmd+J | the same, with a text field of the page in focus |
| system-hide, system-quit | Cmd+H, Cmd+Q | the exception list |
| click-out, given-back | a click out of the plugin, then Q | the host has the keys again |
| click-in, taken | a click in the page, then Q | the plugin has them again |

(F12, F13 and F14 are markers for the driver: they end the run and put the focus into, and out of,
a text field that the driver adds to the page.)

The host window is built as a DAW's is: its content view is an ANCESTOR of the plugin's view, takes every key
without Command as a key command of its own and REFUSES the first responder; a second view takes the focus (the
next key view of the plugin); the window records the keys that end at it; the main menu has key commands. Mouse
clicks are real events posted to the window (a click outside the plugin lands on the content view that refuses
the first responder). Events carry real time stamps and a change of the modifier keys before and after a key with a
modifier, as a keyboard sends them.

Stages `editor-keys` (CLAP, and the CLAP inside the VST3) with the parts:

| Part | What it shows |
| --- | --- |
| `editor-keys-classes` | the keys are the kinds the ticket names (the page's own class of each) and the page saw them in order |
| `editor-keys-host-record` | the host record holds nothing but the exception list, the key after the click outside and one menu action |
| `editor-keys-system` | Cmd+H and Cmd+Q do reach the host (its menu), the page does not see them |
| `editor-keys-space` | the space bar gives exactly one play request and no key; a held space bar gives none; a space in a field types a space and gives none |
| `editor-keys-text` | the text of the group for an absent command's key, none for the other keys |
| `editor-keys-field` | with a text field in focus no key reaches the host; the field takes its characters (also Option+L, 5, 6) and Cmd+Z undoes the typing |
| `editor-keys-focus` | a real click outside gives the keys to the host (the first responder leaves the web view), a real click in the page takes them back |
| `editor-keys-undo` | the page really undid the hit that the key H added (its undo stack 0, 1, 0) |
| `editor-keys-tab` | Tab at the last element of the page does not move the focus to the host's view |
| `editor-keys-action` | a menu action of the host with no target (later than 2 s after the last key) still reaches the host while the plugin has the focus |
| `editor-keys-slow` | with the host's main thread stalled for 400 ms after a key the page does not use, no editing command reaches a host view above the plugin |
| `editor-keys-modifier` | Command down while the host has the keys, a click in the page, Command up: a host VIEW above the plugin gets the release |
| `editor-keys-overlay` | a click on a host view over the plugin gives the keys back; the next key reaches the host |

The steps cover: Space (and held), G, H, U, Cmd+E, Y, Shift+Y, Cmd+J, Option+X, Cmd+Z, Escape, Return, an arrow, F5,
Control+L, Cmd+S, Cmd+W, and in a text field of the page: letter, space, Cmd+E, Cmd+J, Option+L, Option+5,
Option+6, Delete, Cmd+Z, Tab; then the menu action, Cmd+H, Cmd+Q, a click outside and Q, a click in the page and Q.

Stage `editor-keys-twin` (parts `editor-keys-twin-open`, `editor-keys-twin-names`): two copies of the plugin in one
process each open an editor.

Fault runs (`editor-keys-fault`, `editor-keys-twin-fault`), each fails for the named reason:

| Fault | Part that fails | Reason |
| --- | --- | --- |
| `keys-pass` (nothing takes a key) | host-record, field, focus | the keys reached the host |
| `no-shield` (the shield passes an unused key on) | host-record | a key reached the host |
| `no-monitor` | host-record | a key with Command reached the host's menu first |
| `no-focus` (web view not first responder) | host-record | the keys reached the host's view |
| `no-click-out` | focus | the keys did not follow the clicks |
| `no-repeat-guard` | space | a held space bar repeated |
| `no-field-undo` | field | Cmd+Z did not undo |
| `swallow-actions` | action | the shield swallowed the menu action |
| `old-freshness` (round 2 rule) | slow, host-record, field | the command reached the host |
| `no-forward-modifier` | modifier | the release did not reach the host |
| `rect-inside` | overlay | the overlay counted as inside |
| `no-system-keys` | system | a system key did not reach the host |
| `no-key-text` | text | the text is missing |
| the host does not play (`DS_ARA_PLAYBACK_FAULT=skip`) | space | no play request |
| `fixed-class-name` | twin-open | the second copy did not open its editor |
| a run that did not happen | all | no usable run |

### What the test host cannot show, and what no plugin can remove

- A real DAW. The test host is a plain AppKit app. No plugin can stop a key that something sees BEFORE the
  plugin's monitor: a monitor of the host that runs first, an override of `sendEvent:` of the application or the
  window, a layer of the host that reads the keyboard itself, or a hot key of the system (a Carbon or
  event-tap hot key). The plugin cannot know this; the hand check says which route a DAW uses.
- A host that hands its keys to the plugin through `IPlugView::onKeyDown` and `onKeyUp` (see above). The stages
  for the VST3 load the CLAP inside the VST3 bundle directly; they do NOT go through the view of clap-wrapper, so
  the `IPlugView` key calls are not exercised.
- Whether the first responder of the window in a real DAW is the web view after a click in the page. The test
  host posts a real click; a DAW may route the click otherwise.
- The clipboard keys in a text field (no stage touches the pasteboard of the user).

## Limits that the hand check shows

- A menu item that the user picks with the keyboard (the arrow keys and Return in an open menu), or an action from a timer,
  VoiceOver or the Touch Bar within 2 s of a key in the plugin, can be swallowed (the plugin counts it as derived from the key).
  Hand check: press a key in the plugin, then at once choose a menu item of the DAW by keyboard; if it does nothing, this is the cause.
  The rule that needs the key window and an active application is applied to the menu actions only; the test host is not the
  active application, so the stages cannot show it, and the decision is a unit test.
- A title bar accessory view of the host over the content area counts as inside the plugin (the hit test of the content view
  does not see it). A click on it does not give the keys back.
- A host that tracks the modifier keys through its own event monitor (not through its views) does not see a release that
  happens while the plugin has the focus (the plugin drops that event). Hand check: hold Command in the DAW, click into the
  plugin, release, then click in the DAW: if Command seems stuck, this is the cause.

## Open points

- `onKeyDown` and `onKeyUp` of the wrapper (`plugview.cpp`) return `kResultFalse`. A host that asks
  the plugin view first and gets "not handled" runs its key command. A patch of the wrapper could
  return `kResultTrue` for every key while the view is attached (the SDK says a true answer must
  mean the key was handled); then those hosts would not give the page the space bar. Not done here:
  it changes the patched wrapper and a stage cannot reach it (the test host does not call the VST3
  view). To be decided after the hand check in a DAW shows which path the DAW uses.
- A frame of the page that shows who has the keys (see above).
- Hold-down keys: a key that repeats gives one text and many key downs; nothing else is done.

## Hand check in a DAW (on a COPY of a session only)

With the plugin page focused (click in it first), press and watch the session of the DAW. What each result
means is in brackets:

1. A plain key that is a clip command of the DAW (the trim key that started this ticket), a key of an absent
   command of the page (U, C), and a key with no command (Y). The DAW must do nothing. [If the DAW still
   reacts to a plain key, it sees the key before the plugin's monitor (its own monitor or a layer above the
   window), or it uses the `IPlugView` key calls.]
2. Cmd+Z, Cmd+E, Cmd+J and Option+X. The DAW must not undo, export or change anything; the page undoes its own
   hit edit with Cmd+Z. [A DAW menu command that still runs: the DAW takes key equivalents before the plugin's
   monitor.]
3. The space bar: the DAW starts, and stops, once per press; hold it: one change. [Two changes per press: the
   DAW also gets the space as a key.]
4. With a text field of the page focused (the tempo field): type, then Cmd+E, then Tab. The focus stays in the
   page.
5. Click on the DAW's own window (a track, the ruler), then press the same keys: now the DAW must act. [If it
   does not, the click did not take the keys from the page: the DAW consumes the mouse down before the plugin's
   mouse monitor.]
6. Cmd+H and Cmd+Q: the DAW's own commands run (hide; quit asks to save).
