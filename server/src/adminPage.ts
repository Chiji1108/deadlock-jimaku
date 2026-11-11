// 管理画面のページ（DESIGN.md 17.3）。1 枚の HTML で、/api の口だけを使う。ビルドは要らない

export const page = /* html */ `<!doctype html>
<html lang="ja">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex">
<title>デドロ字幕 管理</title>
<style>
  :root { color-scheme: light dark; --bg: #fafafa; --card: #fff; --line: #e4e4e7; --text: #18181b; --muted: #71717a; --accent: #2563eb; --bad: #dc2626; --ok: #16a34a; }
  @media (prefers-color-scheme: dark) { :root { --bg: #09090b; --card: #18181b; --line: #27272a; --text: #fafafa; --muted: #a1a1aa; --accent: #60a5fa; --bad: #f87171; --ok: #4ade80; } }
  * { box-sizing: border-box; }
  body { margin: 0; background: var(--bg); color: var(--text); font: 14px/1.6 system-ui, "Hiragino Sans", "Noto Sans JP", sans-serif; }
  main { max-width: 960px; margin: 0 auto; padding: 24px 16px 64px; display: grid; gap: 16px; }
  header { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
  h1 { font-size: 18px; margin: 0; }
  h2 { font-size: 14px; margin: 0 0 12px; }
  .muted { color: var(--muted); font-size: 12px; }
  .card { background: var(--card); border: 1px solid var(--line); border-radius: 12px; padding: 16px; overflow-x: auto; }
  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 12px; }
  .stat b { display: block; font-size: 22px; font-variant-numeric: tabular-nums; }
  .bar { height: 6px; border-radius: 3px; background: var(--line); margin-top: 6px; overflow: hidden; }
  .bar i { display: block; height: 100%; background: var(--accent); }
  table { width: 100%; border-collapse: collapse; font-variant-numeric: tabular-nums; }
  th, td { text-align: left; padding: 6px 8px; border-bottom: 1px solid var(--line); white-space: nowrap; }
  th { color: var(--muted); font-weight: 500; font-size: 12px; }
  td.num, th.num { text-align: right; }
  button, input, select { font: inherit; color: inherit; }
  button { border: 1px solid var(--line); background: transparent; border-radius: 8px; padding: 4px 10px; cursor: pointer; }
  button:hover { border-color: var(--muted); }
  button.primary { background: var(--accent); border-color: var(--accent); color: #fff; }
  button.danger { color: var(--bad); }
  input, select { border: 1px solid var(--line); background: var(--bg); border-radius: 8px; padding: 6px 8px; min-width: 0; }
  form { display: flex; gap: 8px; flex-wrap: wrap; margin-top: 12px; }
  form input[name=key] { flex: 1 1 240px; }
  .tag { display: inline-block; border-radius: 999px; padding: 0 8px; font-size: 12px; border: 1px solid var(--line); }
  .tag.bad { color: var(--bad); border-color: var(--bad); }
  .tag.ok { color: var(--ok); border-color: var(--ok); }
  #error { color: var(--bad); }
</style>
</head>
<body>
<main>
  <header>
    <h1>デドロ字幕 管理</h1>
    <span class="muted" id="who"></span>
  </header>
  <p id="error" hidden></p>

  <section class="card">
    <h2>ベータ <span class="tag" id="state"></span> <button id="toggle" hidden></button></h2>
    <div class="stats" id="stats"></div>
  </section>

  <section class="card">
    <h2>開発者のキー</h2>
    <p class="muted">会社ごとに複数置けます。断られたキー（使えない、残高がない、回数の上限）は後回しにして、ほかのキーに回します。キーそのものは表示しません</p>
    <table id="keys"></table>
    <form id="add">
      <select name="provider"><option value="qwen">Qwen（シンガポール）</option><option value="openrouter">OpenRouter</option></select>
      <input name="key" type="password" placeholder="API キー" autocomplete="off" required>
      <input name="memo" placeholder="メモ" maxlength="100">
      <button class="primary">足す</button>
    </form>
  </section>

  <section class="card">
    <h2>コード（今日よく使った順）</h2>
    <table id="codes"></table>
  </section>
</main>
<script>
const $ = (id) => document.getElementById(id);
const usd = (n) => "$" + n.toFixed(n < 1 ? 4 : 2);
const date = (ms) => new Date(ms).toLocaleString("ja-JP", { dateStyle: "short", timeStyle: "short" });
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => "&#" + c.charCodeAt(0) + ";");

async function api(path, method = "GET", body) {
  const res = await fetch("/api" + path, {
    method,
    headers: method === "GET" ? {} : { "Content-Type": "application/json", "X-Admin": "1" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) throw new Error(path + ": " + res.status + " " + (await res.text()));
  return res.status === 204 ? undefined : res.json();
}

async function act(run) {
  $("error").hidden = true;
  try { await run(); await load(); } catch (e) { $("error").textContent = e.message; $("error").hidden = false; }
}

async function load() {
  const [s, k, c] = await Promise.all([api("/summary"), api("/keys"), api("/codes")]);
  $("who").textContent = s.email;
  $("state").textContent = s.closed ? "閉じています" : "開いています";
  $("state").className = "tag " + (s.closed ? "bad" : "ok");
  $("toggle").hidden = false;
  $("toggle").textContent = s.closed ? "開ける" : "閉じる";
  $("toggle").onclick = () => act(() => api("/settings", "PUT", { closed: !s.closed }));
  const bar = (v, max) => '<div class="bar"><i style="width:' + Math.min(100, (v / max) * 100) + '%"></i></div>';
  $("stats").innerHTML = [
    ["今日の費用", usd(s.today.usd), "上限 " + usd(s.today.limitUsd) + bar(s.today.usd, s.today.limitUsd)],
    ["今月の費用", usd(s.month.usd), "UTC の月"],
    ["今日出したコード", s.today.codes, "上限 " + s.today.codesLimit + bar(s.today.codes, s.today.codesLimit)],
    ["今日使ったコード", s.codes.activeToday, "全部で " + s.codes.total],
    ["1 つのコードの上限", usd(s.limits.codeDailyUsd), "1 日。同じ所には 1 日 " + s.limits.codesPerPlace + " つ"],
  ].map(([label, value, note]) => '<div class="stat"><span class="muted">' + label + "</span><b>" + value + '</b><span class="muted">' + note + "</span></div>").join("");

  const fallback = Object.entries(k.fallback).filter(([, on]) => on).map(([p]) => p);
  $("keys").innerHTML = "<tr><th>会社</th><th>キー</th><th>メモ</th><th>状態</th><th class=num>断られた回数</th><th>最後に断られた</th><th></th></tr>" +
    k.keys.map((key) => "<tr><td>" + esc(key.provider) + "</td><td>…" + esc(key.tail) + "</td><td>" + esc(key.memo) + "</td><td>" +
      (key.enabled ? '<span class="tag ok">有効</span>' : '<span class="tag">停止</span>') + "</td><td class=num>" + key.failures + "</td><td>" +
      (key.failedAt ? date(key.failedAt) + "（" + key.failedStatus + "）" : "") + '</td><td><button data-key="' + esc(key.id) + '" data-enabled="' + !key.enabled + '">' +
      (key.enabled ? "止める" : "戻す") + '</button> <button class="danger" data-remove="' + esc(key.id) + '">消す</button></td></tr>').join("") +
    fallback.map((p) => "<tr><td>" + esc(p) + '</td><td colspan="6" class="muted">秘密の設定（wrangler）のキー。ほかのキーが断られたときに使います</td></tr>').join("");

  $("codes").innerHTML = "<tr><th>作った日</th><th class=num>今日</th><th class=num>今月</th><th class=num>全部</th><th>状態</th><th></th></tr>" +
    c.codes.map((code) => "<tr><td>" + date(code.createdAt) + "</td><td class=num>" + usd(code.todayUsd) + "</td><td class=num>" + usd(code.monthUsd) + "</td><td class=num>" + usd(code.usd) + "</td><td>" +
      (code.revoked ? '<span class="tag bad">停止</span>' : "") + '</td><td><button data-code="' + esc(code.id) + '" data-revoked="' + !code.revoked + '">' + (code.revoked ? "戻す" : "止める") + "</button></td></tr>").join("");
}

document.addEventListener("click", (e) => {
  const d = e.target.dataset ?? {};
  if (d.key) act(() => api("/keys/" + d.key, "PATCH", { enabled: d.enabled === "true" }));
  if (d.remove && confirm("このキーを消しますか？")) act(() => api("/keys/" + d.remove, "DELETE"));
  if (d.code) act(() => api("/codes/" + d.code, "PATCH", { revoked: d.revoked === "true" }));
});
$("add").addEventListener("submit", (e) => {
  e.preventDefault();
  const form = new FormData(e.target);
  act(async () => { await api("/keys", "POST", Object.fromEntries(form)); e.target.reset(); });
});
act(async () => {});
setInterval(() => load().catch(() => {}), 60000);
</script>
</body>
</html>`;
