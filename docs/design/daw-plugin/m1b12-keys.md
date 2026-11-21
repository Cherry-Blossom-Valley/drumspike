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

Two parts, both in `plugins/ara/src/keys.rs`:

1. **The key shield** (`KeyShield`). The web view is a child of one plain view of the plugin, and
   that view is the child of the host's view. The shield is the next responder above the web view,
   so every key that the page did not use (step 5) arrives at the shield first. It takes key
   down, key up, modifier change, `doCommandBySelector:` and `tryToPerform:with:`, and does not call
   `super`. Nothing goes on to the host's chain.
2. **The equivalent monitor** (`watch_equivalents`). Step 2 is before every view, so a view is too
   late for a key with Command, Control or Option. A local event monitor runs at step 1. While the
   plugin's view has the focus (the first responder of the event's window is the web view or a view
   below the shield), the monitor takes such a key, gives it to the web view as the key down that
   AppKit would send if no menu had taken it (step 3), and drops the event. A key on the exception
   list, and every key while the focus is elsewhere, goes on unchanged. The monitor is made when the
   editor opens and removed when it closes, on the main thread. The clipboard keys (Cmd+C, X, V, A)
   also run the web view's own action: they are menu items in an app, and a text field of the page
   would lose them when the menu of the host cannot get them any more. (Not measured: the stages do
   not touch the pasteboard of the user.)

The shield also gets `performKeyEquivalent:` as a second line (a host without the monitor in
place): it takes the key the same way, except the exception list. AppKit offers EVERY key down to
the views of the window, whoever has the focus, so the shield takes a key only while the plugin's
view has the focus. (The first version did not check this, and the existing stage `editor-focus`
failed: a key after a click in the host still reached the page.)

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

Stages `editor-keys` (CLAP, and the CLAP inside the VST3) with the parts:

| Part | What it shows |
| --- | --- |
| `editor-keys-classes` | the keys are the kinds the ticket names (the page's own class of each) and the page saw them in order |
| `editor-keys-host-record` | the host record holds nothing but the exception list and the key after the click out |
| `editor-keys-system` | Cmd+H and Cmd+Q do reach the host (its menu), the page does not see them |
| `editor-keys-space` | the space bar gives exactly one play request to the host and no key |
| `editor-keys-text` | the text of the group for an absent command's key, none for the other keys |
| `editor-keys-field` | with a text field in focus no key reaches the host, and the field takes its letter |
| `editor-keys-focus` | a click out gives the keys to the host, a click in the page takes them back |

Fault runs (`editor-keys-fault`), each fails for the named reason:

| Fault | Part that fails | Reason |
| --- | --- | --- |
| `keys-pass` (the rule is off: nothing takes a key) | host-record, field, focus | the keys reached the host |
| `no-shield` (the shield passes an unused key on) | host-record | a key reached the host |
| `no-monitor` (no monitor) | host-record | a key with Command reached the host's menu first |
| `no-focus` (the web view is not first responder) | host-record | the keys reached the host's view |
| `no-system-keys` (the system keys are taken too) | system | a system key did not reach the host |
| `no-key-text` (the page shows no text) | text | the text is missing |
| the host does not play (`DS_ARA_PLAYBACK_FAULT=skip`) | space | no play request |
| a run that did not happen | all | no usable run |

### What the test host cannot show

- A real DAW. The test host is a plain AppKit app. A DAW may add its own event monitors, its own
  `NSApplication` or `NSWindow` subclass, a key handler on a layer above the window, or Carbon hot
  keys. Whatever runs before the plugin's monitor sees the key first. The plugin cannot know this.
- A host that hands its keys to the plugin through `IPlugView::onKeyDown` and `onKeyUp` (see above):
  the stages load the CLAP (and the CLAP inside the VST3) directly, not through the wrapper's view.
- Whether the first responder of the window in a real DAW is the web view after a click in the
  page. The test host models it with `makeFirstResponder` and a posted click.
- The clipboard keys in a text field (no stage touches the pasteboard of the user).

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

With the plugin page focused (click in it first), press and watch the session of the DAW:

1. A plain key that is a clip command of the DAW (the trim key that started this ticket), a key of
   an absent command of the page (U, C), and a key with no command (Y).
2. Cmd+Z, Cmd+E, Cmd+J and Option+X. The DAW must not undo, export or change anything; the page
   undoes its own hit edit with Cmd+Z.
3. The space bar: the DAW starts, and stops, once per press.
4. With a text field of the page focused (the tempo field): type, then Cmd+E.
5. Click on the DAW's own window, then press the same keys: now the DAW must act.
6. Cmd+H and Cmd+Q: the DAW's own commands run (hide; quit asks to save).
