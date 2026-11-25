// Drum Spike Deliver: the page. It connects to LUNA, reads the list file and the files that the
// person gives it (File route: works in LUNA and in a browser), shows what a press will do, and
// calls the logic of logic.js. No decision is made here.

const $ = (id) => document.getElementById(id);
const D = globalThis.DrumSpikeDeliver;

let luna;
let session;
let list; // the parsed list, or undefined
let files = new Map(); // file name -> File
let busy = false;
let releases = [];

const setResult = (html, cls = '') => {
  $('result-box').hidden = false;
  const el = $('result');
  el.className = cls;
  el.replaceChildren();
  for (const line of html) {
    const p = document.createElement('div');
    p.textContent = line;
    el.appendChild(p);
  }
};

async function render() {
  const plan = $('plan');
  if (!list) {
    plan.hidden = true;
    $('deliver').disabled = true;
    $('measure').disabled = true;
    return;
  }
  plan.hidden = false;
  const names = D.plannedNames(list);
  const sum = D.summarize(list);
  let problems = [];
  if (session) {
    try {
      const p = await D.preview(session, list, [...files.keys()]);
      if (p.missing.length) problems.push(`Files not chosen yet: ${p.missing.join(', ')}.`);
      if (p.conflicts.length) problems.push(`The session has these tracks already: ${p.conflicts.join(', ')}. A second press would not make a copy; it refuses.`);
    } catch (e) {
      problems.push(e.message);
    }
  } else problems.push('No session is open in LUNA.');
  $('plan-text').textContent = `${sum.tracks} new tracks (${sum.midiTracks} MIDI, ${sum.audioTracks} audio), ${sum.notes} notes, ${sum.files} files. One undo step.`;
  $('plan-names').replaceChildren(
    ...names.map((n) => {
      const li = document.createElement('li');
      li.textContent = n;
      return li;
    }),
  );
  $('plan-problems').textContent = problems.join(' ');
  $('deliver').disabled = busy || problems.length > 0;
  $('measure').disabled = busy || !session;
}

async function take(fileList) {
  const picked = [...fileList];
  const json = picked.find((f) => /\.json$/i.test(f.name));
  for (const f of picked) if (/\.wav$/i.test(f.name)) files.set(f.name, f);
  if (json) {
    try {
      list = D.parseList(await json.text());
      setResult([`Read ${json.name}.`], 'good');
    } catch (e) {
      list = undefined;
      setResult([e.message], 'bad');
    }
  }
  await render();
}

function bindDrop() {
  const drop = $('drop');
  drop.addEventListener('dragover', (e) => {
    e.preventDefault();
    drop.classList.add('hot');
  });
  drop.addEventListener('dragleave', () => drop.classList.remove('hot'));
  drop.addEventListener('drop', (e) => {
    e.preventDefault();
    drop.classList.remove('hot');
    void take(e.dataTransfer.files);
  });
  $('pick').addEventListener('change', (e) => void take(e.target.files));
}

async function press() {
  if (busy || !list) return;
  busy = true;
  await render();
  try {
    const made = await D.deliver(luna, session, list, async (name) => {
      const f = files.get(name);
      if (!f) throw new Error('not chosen');
      return f;
    }, { progress: (text) => setResult([text]) });
    setResult([
      `Made ${made.tracks} tracks: ${[...made.midiTracks, ...made.audioTracks].join(', ')}.`,
      `${made.notes} notes in ${made.midiTracks.length} MIDI clips, ${made.audioTracks.length} audio clips.`,
      'It is ONE undo step: Edit > Undo takes all of it back.',
    ], 'good');
  } catch (e) {
    setResult([e instanceof D.Refused ? e.message : `It failed: ${e.message}. Check the session before you press again.`], 'bad');
  } finally {
    busy = false;
    await render();
  }
}

async function measure() {
  if (busy || !list) return;
  busy = true;
  await render();
  try {
    const report = await D.measure(luna, session, list);
    const text = D.formatReport(report).split('\n');
    setResult(text);
  } catch (e) {
    setResult([`Measure failed: ${e.message}`], 'bad');
  } finally {
    busy = false;
    await render();
  }
}

const keep = (u) => releases.push(u);
function release() {
  for (const u of releases) u();
  releases = [];
}

async function bindSession() {
  release();
  session = luna.sessions.current();
  if (!session) {
    $('status').textContent = 'no session open in LUNA';
    await render();
    return;
  }
  await session.ready();
  $('status').textContent = `Session: ${session.name.value ?? session.uid}`;
  keep(session.tracks.onChange(() => void render()));
  await render();
}

async function connect() {
  for (;;) {
    try {
      luna = await Luna.connect();
      break;
    } catch (e) {
      $('status').textContent = e instanceof Luna.NotRunning ? 'waiting for LUNA...' : e.message;
      await new Promise((r) => setTimeout(r, 1500));
    }
  }
  await luna.sessions.ready();
  luna.sessions.focused.onChange(() => void bindSession());
  luna.sessions.items.onChange(() => void bindSession());
  await bindSession();
}

bindDrop();
$('deliver').onclick = () => void press();
$('measure').onclick = () => void measure();
void connect();
