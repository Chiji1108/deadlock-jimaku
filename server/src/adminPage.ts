// 管理画面のページ（DESIGN.md 17.3）。1 枚の HTML で、/api の口だけを使う。ビルドは要らない

/** ページのスクリプト。応答の CSP に、このスクリプトのハッシュを書く（admin.ts） */
export const script = `const $ = (id) => document.getElementById(id);
const usd = (n) => "$" + n.toFixed(n < 1 ? 4 : 2);
const date = (ms) => new Date(ms).toLocaleString("ja-JP", { dateStyle: "short", timeStyle: "short" });
const version = (v) => (v ? esc(v) : "0.9.6 以前");
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => "&#" + c.charCodeAt(0) + ";");
const tag = (kind, text) => '<span class="tag ' + kind + '">' + text + "</span>";
const bar = (v, max) => '<div class="bar"><i style="width:' + Math.min(100, max > 0 ? (v / max) * 100 : 0) + '%"></i></div>';
const rows = (head, body, empty) => "<thead><tr>" + head + "</tr></thead><tbody>" + (body.join("") || '<tr><td colspan="9" class="muted empty">' + empty + "</td></tr>") + "</tbody>";
/** 古い口（/v1）だけを使う版か: 0.9.9 までと、版を伝えない 0.9.6 まで */
const old = (v) => { const [a, b] = (v ?? "0.0").split(".").map(Number); return a === 0 && b < 10; };

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

/** 開いているタブ。アドレスの # に残す（読み直しても同じタブ）。セクションの id は # と違う名前にして、ブラウザーにその場所まで送らせない */
function show() {
  const tabs = [...document.querySelectorAll("nav a")];
  const now = tabs.find((a) => a.hash === location.hash) ?? tabs[0];
  for (const a of tabs) {
    a.setAttribute("aria-current", a === now ? "page" : "false");
    $("tab-" + a.hash.slice(1)).hidden = a !== now;
  }
  scrollTo(0, 0);
}

async function load() {
  const [s, k, c, t] = await Promise.all([api("/summary"), api("/keys"), api("/codes"), api("/tasks")]);
  $("who").textContent = s.email;
  $("state").textContent = s.closed ? "閉じています" : "開いています";
  $("state").className = "tag " + (s.closed ? "bad" : "ok");
  $("toggle").hidden = false;
  $("toggle").textContent = s.closed ? "開ける" : "閉じる";
  $("toggle").onclick = () => { if (confirm(s.closed ? "ベータを開けますか？" : "ベータを閉じますか？ 新しいコードを出さず、どの仕事も断ります")) act(() => api("/settings", "PUT", { closed: !s.closed })); };
  // 打っている途中の欄は書き換えない（1 分ごとに読み直すため）
  for (const [id, value] of [["dailyUsd", s.today.limitUsd], ["codeDailyUsd", s.limits.codeDailyUsd]]) {
    $(id).max = s.limits.maxUsd;
    if (document.activeElement !== $(id)) $(id).value = value;
  }

  // ---- 概要 ----
  $("stats").innerHTML = [
    ["今日の費用", usd(s.today.usd), "上限 " + usd(s.today.limitUsd) + bar(s.today.usd, s.today.limitUsd)],
    ["今月の費用", usd(s.month.usd), "UTC の月"],
    ["今日使ったコード", s.codes.activeToday, "全部で " + s.codes.total],
    ["今日出したコード", s.today.codes, "上限 " + s.today.codesLimit + bar(s.today.codes, s.today.codesLimit)],
  ].map(([label, value, note]) => '<div class="stat"><span class="muted">' + label + "</span><b>" + value + '</b><span class="muted">' + note + "</span></div>").join("");

  const spent = s.work.rows.reduce((sum, w) => sum + w.usd, 0);
  $("workDays").textContent = s.work.days;
  $("workTotal").textContent = spent > 0 ? "合計 " + usd(spent) : "";
  $("work").innerHTML = rows("<th>仕事</th><th>モデル</th><th class=num>額</th><th class=share>割合</th><th class=num>回数</th><th class=num>1 回あたり</th>",
    s.work.rows.map((w) => "<tr><td>" + esc(w.label) + '</td><td class="muted">' + esc(w.modelLabel) + "</td><td class=num>" + usd(w.usd) + '</td><td class=share><span>' + Math.round((w.usd / spent) * 100) + "%</span>" + bar(w.usd, spent) +
      "</td><td class=num>" + w.count.toLocaleString() + "</td><td class=num>$" + (w.usd / w.count).toFixed(6) + "</td></tr>"),
    "まだ記録がありません。仕事を頼まれると、ここに出ます");

  // ---- モデル ----
  // 選んでいる途中の欄は書き換えない（1 分ごとに読み直すため）
  if (!document.activeElement?.dataset?.model) $("tasks").innerHTML = t.tasks.map((task) => {
    const cost = s.work.rows.find((w) => w.task === task.name && w.model === task.model);
    const state = !task.enabled ? tag("bad", "止めている") : task.keyed ? tag("ok", "動かしている") : tag("bad", "キーがないので頼めない");
    return '<div class="task"><div class="row"><b>' + esc(task.label) + "</b>" + state + '</div><select data-model="' + esc(task.name) + '">' +
      task.models.map((m) => '<option value="' + esc(m.id) + '"' + (m.id === task.model ? " selected" : m.keyed ? "" : " disabled") + ">" + esc(m.label) + (m.keyed ? "" : "（キーがない）") + "</option>").join("") +
      '</select><div class="row"><span class="muted">' + (cost ? "この " + s.work.days + " 日 " + usd(cost.usd) + "・1 回 $" + (cost.usd / cost.count).toFixed(6) : "このモデルの記録はまだありません") +
      '</span><button data-task="' + esc(task.name) + '" data-enabled="' + !task.enabled + '">' + (task.enabled ? "止める" : "動かす") + "</button></div></div>";
  }).join("");

  // ---- キー ----
  const fallback = Object.entries(k.fallback).filter(([, on]) => on).map(([p]) => p);
  $("keys").innerHTML = rows("<th>会社</th><th>キー</th><th>メモ</th><th>状態</th><th class=num>断られた回数</th><th>最後に断られた</th><th></th>",
    k.keys.map((key) => "<tr><td>" + esc(key.provider) + '</td><td class="mono">…' + esc(key.tail) + "</td><td>" + esc(key.memo) + "</td><td>" +
      (key.enabled ? tag("ok", "有効") : tag("", "停止")) + "</td><td class=num>" + key.failures + "</td><td>" +
      (key.failedAt ? date(key.failedAt) + "（" + (key.failedStatus === 0 ? "読めない" : key.failedStatus) + "）" : "") +
      (key.failedReason ? '<div class="muted reason">' + esc(key.failedReason) + "</div>" : "") + '</td><td class="acts"><button data-key="' + esc(key.id) + '" data-enabled="' + !key.enabled + '">' +
      (key.enabled ? "止める" : "戻す") + '</button> <button class="danger" data-remove="' + esc(key.id) + '">消す</button></td></tr>')
    .concat(fallback.map((p) => "<tr><td>" + esc(p) + '</td><td colspan="6" class="muted">秘密の設定（wrangler）のキー。ほかのキーが断られたときに使います</td></tr>')),
    "キーがありません。下から足すと、ベータが開きます");

  // ---- 利用者 ----
  $("codeCount").textContent = "今日 " + s.codes.activeToday + "・全部で " + s.codes.total;
  $("codes").innerHTML = rows("<th>作った日</th><th>最後に来た日</th><th>版</th><th class=num>今日</th><th class=num>今月</th><th class=num>全部</th><th>状態</th><th></th>",
    c.codes.map((code) => "<tr><td>" + date(code.createdAt) + "</td><td>" + (code.seenDay ? esc(code.seenDay) : "") + "</td><td>" + (code.seenDay ? version(code.version) : "") + "</td><td class=num>" + usd(code.todayUsd) + "</td><td class=num>" + usd(code.monthUsd) + "</td><td class=num>" + usd(code.usd) + "</td><td>" +
      (code.revoked ? tag("bad", "停止") : "") + '</td><td class="acts"><button data-code="' + esc(code.id) + '" data-revoked="' + !code.revoked + '">' + (code.revoked ? "戻す" : "止める") + "</button></td></tr>"),
    "まだコードがありません");

  // ---- アプリの版 ----
  const behind = s.versions.filter((v) => old(v.version));
  const count = behind.reduce((sum, v) => sum + v.codes, 0);
  $("oldCodes").innerHTML = count === 0 ? "古い口を使う版は、もう来ていません" :
    "<b>" + count + "</b> 個のコードが、古い口を使う版です。最後に来たのは " + esc(behind.map((v) => v.lastDay).sort().pop());
  $("versions").innerHTML = rows("<th>版</th><th>頼む口</th><th>最後に来た日</th><th class=num>コード</th>",
    s.versions.map((v) => "<tr><td>" + version(v.version) + (v.refused ? " " + tag("bad", "断っている") : "") + '</td><td class="muted">' + (old(v.version) ? "古い口（/v1）" : "新しい口（/v2）") + "</td><td>" + esc(v.lastDay) + "</td><td class=num>" + v.codes + "</td></tr>"),
    "まだ、どの版も来ていません");
  // 打っている途中の欄は書き換えない
  if (document.activeElement !== $("minVersion")) $("minVersion").value = s.minVersion;
  $("minState").innerHTML = s.minVersion ? tag("bad", esc(s.minVersion) + " より前を断っている") : tag("", "どの版も断っていない");
}

document.addEventListener("click", (e) => {
  const d = e.target.dataset ?? {};
  if (d.key) act(() => api("/keys/" + d.key, "PATCH", { enabled: d.enabled === "true" }));
  if (d.remove && confirm("このキーを消しますか？")) act(() => api("/keys/" + d.remove, "DELETE"));
  if (d.code) act(() => api("/codes/" + d.code, "PATCH", { revoked: d.revoked === "true" }));
  if (d.task) act(() => api("/tasks/" + d.task, "PATCH", { enabled: d.enabled === "true" }));
});
document.addEventListener("change", (e) => {
  const task = e.target.dataset?.model;
  if (task) { e.target.blur(); act(() => api("/tasks/" + task, "PATCH", { model: e.target.value })); }
});
$("limits").addEventListener("submit", (e) => {
  e.preventDefault();
  act(() => api("/settings", "PUT", { dailyUsd: Number($("dailyUsd").value), codeDailyUsd: Number($("codeDailyUsd").value) }));
});
$("oldest").addEventListener("submit", (e) => {
  e.preventDefault();
  const min = $("minVersion").value.trim();
  const ask = min ? min + " より前の版を断ります（版を伝えない 0.9.6 以前も断ります）。よいですか？" : "どの版も断らないようにします。よいですか？";
  if (confirm(ask)) act(() => api("/settings", "PUT", { minVersion: min }));
});
$("add").addEventListener("submit", (e) => {
  e.preventDefault();
  const form = new FormData(e.target);
  act(async () => { await api("/keys", "POST", Object.fromEntries(form)); e.target.reset(); });
});
addEventListener("hashchange", show);
show();
act(async () => {});
setInterval(() => load().catch(() => {}), 60000);
`;

export const page = /* html */ `<!doctype html>
<html lang="ja">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex">
<title>デドロ字幕 管理</title>
<style>
  :root { color-scheme: light dark; --bg: #f6f6f7; --card: #fff; --line: #e4e4e7; --soft: #f4f4f5; --text: #18181b; --muted: #71717a; --accent: #2563eb; --bad: #dc2626; --ok: #16a34a; }
  @media (prefers-color-scheme: dark) { :root { --bg: #09090b; --card: #151518; --line: #27272a; --soft: #1f1f23; --text: #fafafa; --muted: #a1a1aa; --accent: #60a5fa; --bad: #f87171; --ok: #4ade80; } }
  * { box-sizing: border-box; }
  [hidden] { display: none !important; }
  body { margin: 0; background: var(--bg); color: var(--text); font: 14px/1.6 system-ui, "Hiragino Sans", "Noto Sans JP", sans-serif; }
  .top { position: sticky; top: 0; z-index: 1; background: color-mix(in srgb, var(--bg) 88%, transparent); backdrop-filter: blur(8px); border-bottom: 1px solid var(--line); }
  .top > div, main { max-width: 1040px; margin: 0 auto; padding: 0 16px; }
  header { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; padding-top: 14px; }
  header .who { margin-left: auto; }
  h1 { font-size: 16px; margin: 0; }
  h2 { font-size: 14px; margin: 0; }
  nav { display: flex; gap: 4px; margin-top: 10px; overflow-x: auto; }
  nav a { padding: 8px 12px; color: var(--muted); text-decoration: none; white-space: nowrap; border-bottom: 2px solid transparent; margin-bottom: -1px; }
  nav a:hover { color: var(--text); }
  nav a[aria-current=page] { color: var(--text); font-weight: 600; border-color: var(--accent); }
  main { padding-top: 20px; padding-bottom: 64px; }
  main > section { display: grid; grid-template-columns: minmax(0, 1fr); gap: 16px; }
  .muted { color: var(--muted); font-size: 12px; }
  .mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
  .reason { max-width: 360px; overflow-wrap: anywhere; white-space: normal; }
  .card { background: var(--card); border: 1px solid var(--line); border-radius: 12px; padding: 16px; }
  .card > .head { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; flex-wrap: wrap; margin-bottom: 4px; }
  .card > p { margin: 0 0 12px; max-width: 88ch; }
  .scroll { overflow-x: auto; margin: 0 -16px; padding: 0 16px; }
  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(170px, 1fr)); gap: 12px; }
  .stat { background: var(--card); border: 1px solid var(--line); border-radius: 12px; padding: 14px 16px; }
  .stat b { display: block; font-size: 24px; line-height: 1.3; font-variant-numeric: tabular-nums; }
  .bar { height: 4px; border-radius: 2px; background: var(--soft); margin-top: 6px; overflow: hidden; }
  .bar i { display: block; height: 100%; background: var(--accent); border-radius: 2px; }
  table { width: 100%; border-collapse: collapse; font-variant-numeric: tabular-nums; }
  th, td { text-align: left; padding: 8px; border-bottom: 1px solid var(--line); white-space: nowrap; vertical-align: top; }
  tbody tr:last-child td { border-bottom: 0; }
  th { color: var(--muted); font-weight: 500; font-size: 12px; }
  td.num, th.num { text-align: right; }
  td.share { min-width: 120px; } td.share span { font-size: 12px; }
  td.acts { text-align: right; }
  td.empty { text-align: center; padding: 24px 8px; }
  button, input, select { font: inherit; color: inherit; }
  button { border: 1px solid var(--line); background: var(--card); border-radius: 8px; padding: 4px 10px; cursor: pointer; }
  button:hover { border-color: var(--muted); }
  button.primary { background: var(--accent); border-color: var(--accent); color: #fff; }
  button.danger { color: var(--bad); }
  input, select { border: 1px solid var(--line); background: var(--bg); border-radius: 8px; padding: 6px 8px; min-width: 0; }
  :focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  form { display: flex; gap: 8px; flex-wrap: wrap; align-items: end; }
  form label { display: grid; gap: 2px; font-size: 12px; color: var(--muted); }
  form label input { color: var(--text); font-size: 14px; }
  form input[name=key] { flex: 1 1 240px; }
  .tasks { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 12px; }
  .task { background: var(--card); border: 1px solid var(--line); border-radius: 12px; padding: 16px; display: grid; gap: 10px; }
  .task select { width: 100%; }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .tag { display: inline-block; border-radius: 999px; padding: 0 8px; font-size: 12px; font-weight: 400; border: 1px solid var(--line); color: var(--muted); white-space: nowrap; }
  .tag.bad { color: var(--bad); border-color: var(--bad); }
  .tag.ok { color: var(--ok); border-color: var(--ok); }
  dl { display: grid; grid-template-columns: max-content 1fr; gap: 6px 16px; margin: 0; }
  dt { color: var(--muted); } dd { margin: 0; }
  #error { color: var(--bad); background: var(--card); border: 1px solid var(--bad); border-radius: 12px; padding: 10px 16px; margin: 0 0 16px; overflow-wrap: anywhere; }
</style>
</head>
<body>
<div class="top"><div>
  <header>
    <h1>デドロ字幕 管理</h1>
    <span class="tag" id="state"></span>
    <button id="toggle" hidden></button>
    <span class="muted who" id="who"></span>
  </header>
  <nav>
    <a href="#overview">概要</a>
    <a href="#models">モデル</a>
    <a href="#providers">キー</a>
    <a href="#users">利用者</a>
    <a href="#apps">アプリの版</a>
  </nav>
</div></div>
<main>
  <p id="error" hidden></p>

  <section id="tab-overview">
    <div class="stats" id="stats"></div>
    <div class="card">
      <div class="head"><h2>費用の内訳（この <span id="workDays"></span> 日）</h2><span class="muted" id="workTotal"></span></div>
      <p class="muted">仕事とモデルごとの額です（UTC の日で数えます）。回数は、料金のかかった要求の数。翻訳は、翻訳先がいくつでも 1 回です。コードごとには分けません</p>
      <div class="scroll"><table id="work"></table></div>
    </div>
    <div class="card">
      <div class="head"><h2>上限</h2></div>
      <p class="muted">上限に届くと、その日は仕事を断ります（アプリは Chrome の作り方で字幕を続けます）。1 つのコードの上限は、出してあるコードにも当てます</p>
      <form id="limits">
        <label>全体の 1 日の上限（ドル）<input id="dailyUsd" type="number" min="0.01" step="0.01" required></label>
        <label>1 つのコードの 1 日の上限（ドル）<input id="codeDailyUsd" type="number" min="0.01" step="0.01" required></label>
        <button>上限を変える</button>
      </form>
    </div>
  </section>

  <section id="tab-models" hidden>
    <p class="muted">アプリは「何をしてほしいか」だけを送り、モデルはここで決めます。替えると、次の要求から効きます（アプリの更新は要りません）。止めた仕事は、その工程だけ断ります（アプリは、ほかの工程を続けます）。古い口を使う版（0.9.9 まで）には効きません。0.10.0 のアプリは、読みも「用語を選ぶ」のモデルに頼みます</p>
    <div class="tasks" id="tasks"></div>
  </section>

  <section id="tab-providers" hidden>
    <div class="card">
      <div class="head"><h2>開発者のキー</h2></div>
      <p class="muted">会社ごとに複数置けます。断られたキー（使えない、残高がない、回数の上限）は後回しにして、ほかのキーに回します。キーそのものは表示しません</p>
      <div class="scroll"><table id="keys"></table></div>
    </div>
    <div class="card">
      <div class="head"><h2>キーを足す</h2></div>
      <form id="add">
        <select name="provider"><option value="qwen">Qwen（シンガポール）</option><option value="openrouter">OpenRouter</option><option value="perplexity">Perplexity</option><option value="openai">OpenAI</option></select>
        <input name="key" type="password" placeholder="API キー" autocomplete="off" required>
        <input name="memo" placeholder="メモ" maxlength="100">
        <button class="primary">足す</button>
      </form>
    </div>
  </section>

  <section id="tab-users" hidden>
    <div class="card">
      <div class="head"><h2>コード（今日よく使った順）</h2><span class="muted" id="codeCount"></span></div>
      <p class="muted">アプリが自動でもらうコードです。誰のものかは分かりません。止めたコードは、仕事を頼めなくなります</p>
      <div class="scroll"><table id="codes"></table></div>
    </div>
  </section>

  <section id="tab-apps" hidden>
    <div class="card">
      <div class="head"><h2>来ている版</h2></div>
      <p class="muted">アプリが確かめに来たコードの数と、その版が最後に来た日（UTC）です（止めたコードは除く）。アプリは、字幕を始めるときと「おすすめ」を選ぶときにコードを確かめ、版を伝えます（0.9.7 から）</p>
      <div class="scroll"><table id="versions"></table></div>
    </div>
    <div class="card">
      <div class="head"><h2>古い口（/v1）</h2></div>
      <p class="muted">0.9.9 までのアプリは、モデルを自分で決めて古い口に頼みます。「モデル」タブで替えても効かず、下のモデルに決まっています。使う版が来なくなったら、この口をやめられます</p>
      <p id="oldCodes"></p>
      <dl>
        <dt>翻訳</dt><dd>Hy-MT2（OpenRouter）</dd>
        <dt>用語と読みを選ぶ</dt><dd>Perplexity Decider V1.2（OpenRouter）</dd>
        <dt>確定の文字起こし</dt><dd>Qwen-Audio-3.1-ASR-Flash</dd>
        <dt>途中の文字起こし</dt><dd>Qwen-Audio-3.1-ASR-Flash-Streaming</dd>
      </dl>
    </div>
    <div class="card">
      <div class="head"><h2>古い版を断る</h2><span id="minState"></span></div>
      <p class="muted">決めた版より前のアプリには、字幕を始めるときの確かめで「更新が要る」と答えます。0.9.7 からのアプリは「アプリを更新してください」と出して、サーバーを使わない作り方で字幕を続けます。0.9.6 以前は更新が要ることを伝えられないので、使う人がいなくなってから決めます。空にすると、どの版も断りません</p>
      <form id="oldest">
        <label>この版より前は断る<input id="minVersion" placeholder="0.9.7" pattern="\\d+\\.\\d+\\.\\d+(-[0-9A-Za-z.]+)?" size="12"></label>
        <button>決める</button>
      </form>
    </div>
  </section>
</main>
<script>${script}</script>
</body>
</html>`;
