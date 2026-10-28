/* Self-test probe driver (#206 diagnosis, throwaway): for a few keys it sends synthetic keyDown
   variants through `selftest_menu_probe` and reports which menu id each one fired (the delta of
   the Rust-side `selftest_menu_log`). No project needed. */
(async () => {
  const T = window.__TAURI__.core;
  const report = { scenario: "probe", pass: true, cases: [] };
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  const logLen = async () => (await T.invoke("selftest_menu_log")).length;
  for (const c of [
    { name: "Shift+Cmd+O", variants: [["O", 1179648], ["o", 1179648], ["O", 1572864], ["o", 1572864], ["o", 1048576]] },
    { name: "Cmd+I", variants: [["i", 1048576], ["I", 1048576], ["i", 1179648], ["I", 1179648]] },
    { name: "Option+Shift+M", variants: [["m", 655360], ["M", 655360], ["m", 524288], ["M", 524288]] },
    { name: "Cmd+O", variants: [["o", 1048576], ["O", 1048576]] },
  ]) {
    const out = [];
    for (const [chars, flags] of c.variants) {
      const n0 = await logLen();
      const r = await T.invoke("selftest_menu_probe", { chars, flags });
      await sleep(700);
      const delta = (await T.invoke("selftest_menu_log")).slice(n0);
      out.push({ chars, flags, matched: r.matched, fired: delta });
    }
    report.cases.push({ key: c.name, variants: out });
  }
  await T.invoke("selftest_report", { report });
})();
