// Chrome の翻訳（Translator）と認識（Web Speech）を、ほかの方式と同じ文と録音で測る（DESIGN.md 9.7）。
// アプリと同じく、別のプロファイルで Chrome を開いて CDP で動かす。認識には、録音を偽のマイクとして聞かせる。
//
// usage: node chrome_eval.mjs translate GLOSSARY.json PROFILE_DIR
//        node chrome_eval.mjs speech data/sample01.wav data/sample01.txt PROFILE_DIR
//        node chrome_eval.mjs furigana PROFILE_DIR [REPEAT]
//        node chrome_eval.mjs choose PROFILE_DIR QUESTIONS.json CASES.json [REPEAT]
//        node chrome_eval.mjs decider PROFILE_DIR
//   decider:   Gemini Nano を、判定モデルと同じ形の口で待たせる（用語の判定を `split_eval terms-jev local-gemini-nano` で測る）
//   choose:    読みが分かれる語を、辞書の候補から Gemini Nano に選ばせる（下の `choose` の説明）
//   furigana:  data/furigana_cases.json の文の、辞書の読みを Gemini Nano（Prompt API）に直させる。文は PC から出ない
//   translate: results/terms_cases.json の文を、用語を英語名に置き換えて（アプリが Chrome に渡す形）英語とロシア語に訳す
//   speech:    録音を 3 回聞かせ、確定した文をつなげて文字誤り率を見る。音声は Google に送られる
import { spawn } from "node:child_process";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { createServer } from "node:http";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const TARGETS = ["en", "ru"];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Translator は安全なページでしか使えないので、空のページを localhost で出す */
async function serve() {
  const server = createServer((_, res) => res.end("<!doctype html><title>eval</title>"));
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return server;
}

async function open(profile, flags) {
  const server = await serve();
  const url = `http://localhost:${server.address().port}/`;
  const chrome = spawn(
    CHROME,
    [`--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check", ...flags, url],
    { stdio: "ignore" },
  );
  const portFile = join(profile, "DevToolsActivePort");
  for (let i = 0; i < 100 && !existsSync(portFile); i++) await sleep(100);
  await sleep(1000);
  const port = readFileSync(portFile, "utf8").split("\n")[0];
  const pages = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
  const ws = new WebSocket(pages.find((p) => p.url === url).webSocketDebuggerUrl);
  await new Promise((r) => (ws.onopen = r));
  let id = 0;
  const waiting = new Map();
  ws.onmessage = ({ data }) => {
    const m = JSON.parse(data);
    waiting.get(m.id)?.(m);
  };
  /** ページの中で動かす。クリックの中として扱わせる（モデルのダウンロードに要る） */
  const evaluate = async (expression) => {
    const m = await new Promise((r) => {
      waiting.set(++id, r);
      const params = { expression, awaitPromise: true, returnByValue: true, userGesture: true };
      ws.send(JSON.stringify({ id, method: "Runtime.evaluate", params }));
    });
    if (m.error || m.result.exceptionDetails) throw new Error(JSON.stringify(m.error ?? m.result.exceptionDetails));
    return m.result.result.value;
  };
  const close = () => {
    chrome.kill();
    server.close();
  };
  return { evaluate, close };
}

const kept = (translation, name) =>
  new RegExp(`(?<![0-9A-Za-z])${name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(?![0-9A-Za-z])`).test(translation);

async function translate(glossaryPath, profile) {
  const { entities } = JSON.parse(readFileSync(glossaryPath, "utf8"));
  const cases = JSON.parse(readFileSync(join(HERE, "results/terms_cases.json"), "utf8"));
  const { evaluate, close } = await open(profile, []);
  // 言語ごとに分けて作る（ダウンロードにはクリックが要り、1 回のクリックは 1 回しか使えない）
  await evaluate("window.translators = {}");
  for (const target of TARGETS)
    await evaluate(`Translator.create({ sourceLanguage: "ja", targetLanguage: "${target}" })
      .then((t) => { window.translators["${target}"] = t; })`);
  const runs = [];
  for (const c of cases) {
    const segments = c.llm[0].answer.segments;
    const names = segments.filter((s) => s.ref).map((s) => entities[s.ref].names.en);
    const text = segments.map((s) => (s.ref ? entities[s.ref].names.en : s.text)).join("");
    const { ms, translations } = await evaluate(`(async () => {
      const t0 = performance.now();
      const entries = await Promise.all(Object.entries(window.translators).map(
        async ([lang, t]) => [lang, await t.translate(${JSON.stringify(text)})]));
      return { ms: Math.round(performance.now() - t0), translations: Object.fromEntries(entries) };
    })()`);
    const missing = Object.fromEntries(
      Object.entries(translations).map(([lang, t]) => [lang, names.filter((n) => !kept(t, n))]),
    );
    runs.push({ text: c.text, sent: text, ms, ok: Object.values(missing).every((m) => m.length === 0), translations, missing });
  }
  close();
  const times = runs.map((r) => r.ms).sort((a, b) => a - b);
  const summary = {
    runs: runs.length,
    ok: runs.filter((r) => r.ok).length,
    medianMs: times[Math.floor(times.length / 2)],
    p90Ms: times[Math.floor((times.length * 9) / 10)],
  };
  const out = join(HERE, "results/local_translate_eval.json");
  const report = JSON.parse(readFileSync(out, "utf8"));
  report["chrome-translator"] = { summary, runs };
  writeFileSync(out, JSON.stringify(report, null, 1) + "\n");
  console.log("chrome-translator", JSON.stringify(summary));
}

const normalize = (s) => s.replace(/[\s、。，．,.!?！？「」…〜]/g, "");
function cer(ref, hyp) {
  const [a, b] = [normalize(ref), normalize(hyp)];
  let prev = Array.from({ length: b.length + 1 }, (_, i) => i);
  for (let i = 1; i <= a.length; i++) {
    const cur = [i];
    for (let j = 1; j <= b.length; j++) cur.push(Math.min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (a[i - 1] !== b[j - 1])));
    prev = cur;
  }
  return prev[b.length] / Math.max(a.length, 1);
}

async function speech(wav, refPath, profile) {
  const ref = readFileSync(refPath, "utf8").trim();
  const seconds = (readFileSync(wav).length - 44) / 32000;
  const runs = [];
  for (let i = 0; i < 3; i++) {
    const { evaluate, close } = await open(profile, [
      // 音声のプロセスがサンドボックスの中だと、録音のファイルを読めず無音になる（macOS）
      "--disable-features=AudioServiceSandbox,AudioServiceOutOfProcess",
      "--use-fake-ui-for-media-stream",
      "--use-fake-device-for-media-stream",
      `--use-file-for-fake-audio-capture=${resolve(wav)}%noloop`,
    ]);
    // アプリと同じ設定（日本語、聞き続ける、途中の文も受ける）。録音が終わって少し待ってから止める
    const run = await evaluate(`(async () => {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const recognizer = new webkitSpeechRecognition();
      recognizer.lang = "ja-JP";
      recognizer.continuous = true;
      recognizer.interimResults = true;
      const finals = [], errors = [];
      let interim = "";
      recognizer.onresult = (e) => {
        interim = "";
        for (let i = e.resultIndex; i < e.results.length; i++) {
          const text = e.results[i][0].transcript;
          if (e.results[i].isFinal) finals.push(text); else interim += text;
        }
      };
      recognizer.onerror = (e) => errors.push(e.error);
      const ended = new Promise((r) => (recognizer.onend = r));
      recognizer.start(stream.getAudioTracks()[0]);
      await new Promise((r) => setTimeout(r, ${Math.round((seconds + 6) * 1000)}));
      recognizer.stop();
      await ended;
      return { text: finals.join("") + interim, finals, errors };
    })()`);
    close();
    runs.push({ ...run, cer: Math.round(cer(ref, run.text) * 10000) / 10000 });
    console.log(`CER ${(runs.at(-1).cer * 100).toFixed(1)}%  ${run.errors}\n  ${run.text}`);
    await sleep(1500);
  }
  const name = wav.split("/").pop().replace(/\.wav$/, "");
  writeFileSync(join(HERE, `results/${name}.chrome.json`), JSON.stringify({ "chrome-web-speech": runs }, null, 2) + "\n");
}

const LANGUAGES = {
  expectedInputs: [{ type: "text", languages: ["en", "ja"] }],
  expectedOutputs: [{ type: "text", languages: ["ja"] }],
};
const FURIGANA_SYSTEM =
  "You check the readings (furigana) of a Japanese subtitle from a video game streamer. " +
  'The input is JSON: "sentence" is the subtitle, and "words" lists its words that contain kanji, each with the reading a dictionary gave, in hiragana. ' +
  "The dictionary reads each word without looking at the sentence, so some readings are wrong for this sentence (for example 辛い as つらい when it means spicy, or 一 and 人 as いち and にん when the sentence says ひとり). " +
  'Answer with JSON {"readings": [...]}: the correct hiragana reading of each word, in the same order and the same number as "words". ' +
  "Keep the dictionary's reading unless it is wrong in this sentence. A reading covers only the characters of its word.";

/** 辞書（Lindera）の読みを Gemini Nano（Prompt API）に直させて、直せた文と壊した文を数える（DESIGN.md 9.3） */
async function furigana(profile, repeat = "3") {
  const cases = JSON.parse(readFileSync(join(HERE, "data/furigana_cases.json"), "utf8"));
  const { evaluate, close } = await open(profile, []);
  const availability = await evaluate(
    `"LanguageModel" in window ? LanguageModel.availability(${JSON.stringify(LANGUAGES)}) : "no LanguageModel"`,
  );
  console.log("availability:", availability);
  if (availability === "unavailable" || availability === "no LanguageModel") return close();
  // モデルがなければ、ここでダウンロードする（数 GB）
  const t0 = Date.now();
  await evaluate(`(async () => {
    window.progress = 0;
    window.base = await LanguageModel.create({
      ...${JSON.stringify(LANGUAGES)},
      initialPrompts: [{ role: "system", content: ${JSON.stringify(FURIGANA_SYSTEM)} }],
      monitor(m) { m.addEventListener("downloadprogress", (e) => { window.progress = e.loaded; }); },
    });
  })()`);
  console.log(`ready in ${Math.round((Date.now() - t0) / 1000)} s`);
  const right = (words, readings) => words.every((w, i) => (w.accept ?? [w.reading]).includes(readings[i]));
  const runs = [];
  for (const c of cases) {
    const words = c.words.filter((w) => w.reading);
    const schema = {
      type: "object",
      required: ["readings"],
      additionalProperties: false,
      properties: { readings: { type: "array", minItems: words.length, maxItems: words.length, items: { type: "string" } } },
    };
    const input = JSON.stringify({ sentence: c.text, words: words.map(({ text, reading }) => ({ text, reading })) });
    const dictionary = right(words, words.map((w) => w.reading));
    for (let i = 0; i < Number(repeat); i++) {
      // 文ごとに新しい会話にする（前の文を引きずらない）
      const { ms, answer } = await evaluate(`(async () => {
        const session = await window.base.clone();
        const t0 = performance.now();
        const answer = await session.prompt(${JSON.stringify(input)}, { responseConstraint: ${JSON.stringify(schema)} });
        const ms = Math.round(performance.now() - t0);
        session.destroy();
        return { ms, answer };
      })()`);
      let readings = [];
      try {
        readings = JSON.parse(answer).readings ?? [];
      } catch {}
      // 形が合わない答えは使えない（アプリなら辞書の読みに戻す）
      const usable = readings.length === words.length && readings.every((r) => /^[ぁ-ゖー]+$/.test(r));
      const nano = usable ? right(words, readings) : dictionary;
      const changed = usable ? words.flatMap((w, j) => (readings[j] === w.reading ? [] : [`${w.text}: ${w.reading} → ${readings[j]}`])) : [];
      runs.push({ text: c.text, ms, usable, dictionary, nano, changed });
      if (dictionary !== nano || !usable) console.log(!usable ? "unusable" : nano ? "fixed " : "broken", c.text, changed.join(", "), usable ? "" : answer);
    }
  }
  close();
  const times = runs.map((r) => r.ms).sort((a, b) => a - b);
  const count = (f) => runs.filter(f).length;
  const summary = {
    runs: runs.length,
    dictionaryRight: count((r) => r.dictionary),
    nanoRight: count((r) => r.nano),
    fixed: count((r) => !r.dictionary && r.nano),
    broken: count((r) => r.dictionary && !r.nano),
    stillWrong: count((r) => !r.dictionary && !r.nano),
    unusable: count((r) => !r.usable),
    medianMs: times[Math.floor(times.length / 2)],
    p90Ms: times[Math.floor((times.length * 9) / 10)],
  };
  writeFileSync(join(HERE, "results/furigana_nano.json"), JSON.stringify({ system: FURIGANA_SYSTEM, summary, runs }, null, 1) + "\n");
  console.log("gemini-nano", JSON.stringify(summary));
}

const CHOOSE_SYSTEM =
  "You decide how a word is read aloud in a Japanese subtitle from a streamer playing the video game Deadlock. " +
  'The input is JSON: "sentence" is the subtitle, "word" is the part in question (with the kana that follow it), ' +
  'and "readings" are the possible readings in hiragana, each under a number. ' +
  'Answer with JSON {"reading": ...}: the number of the one reading that fits the meaning of this sentence.';

/**
 * 読みが分かれる語を、辞書の候補から Gemini Nano（Prompt API）に選ばせる（DESIGN.md 6.4）。候補はアプリが判定モデルに
 * 選ばせるものと同じ（`cargo run --release -p overlays-core --example readings -- CASES.json > QUESTIONS.json`）。
 * CASES.json に語ごとの正しい読みがあれば（`data/furigana_cases.json`）それと比べ、なければ辞書の読みを正しいとみなす
 */
async function choose(profile, questionsPath, casesPath, repeat = "3") {
  const questions = JSON.parse(readFileSync(questionsPath, "utf8"));
  const cases = JSON.parse(readFileSync(casesPath, "utf8"));
  const hira = (s) => s.replace(/[ァ-ヶ]/g, (c) => String.fromCharCode(c.charCodeAt(0) - 0x60));
  // 文全体の正しい読み（語ごとの候補の組み合わせ）
  const gold = (c) => {
    if (!Array.isArray(c.words)) return undefined;
    let all = [""];
    for (const w of c.words) {
      const alts = w.reading ? (w.accept ?? [w.reading]) : [w.text];
      all = all.flatMap((head) => alts.map((a) => head + hira(a)));
    }
    return new Set(all);
  };
  const { evaluate, close } = await open(profile, []);
  const availability = await evaluate(
    `"LanguageModel" in window ? LanguageModel.availability(${JSON.stringify(LANGUAGES)}) : "no LanguageModel"`,
  );
  console.log("availability:", availability);
  if (availability === "unavailable" || availability === "no LanguageModel") return close();
  await evaluate(`(async () => {
    window.base = await LanguageModel.create({
      ...${JSON.stringify(LANGUAGES)},
      initialPrompts: [{ role: "system", content: ${JSON.stringify(CHOOSE_SYSTEM)} }],
    });
  })()`);
  const runs = [];
  const times = [];
  for (const [i, q] of questions.entries()) {
    const read = (words) => hira(words.map((w) => w.chosen ?? w.reading ?? w.text).join(""));
    const own = read(q.words);
    const right = gold(cases[i]) ?? new Set([own]);
    for (let n = 0; n < Number(repeat); n++) {
      const words = [];
      for (const w of q.words) {
        if (!w.options) {
          words.push(w);
          continue;
        }
        // アプリと同じく、続くかなを付けて見せる
        const readings = w.options.map((o) => o + w.tail);
        // 答えは候補の番号（日本語の文字を答えの決まりに入れると、Prompt API がエラーを返した）
        const numbers = readings.map((_, n) => String(n + 1));
        const schema = { type: "object", required: ["reading"], additionalProperties: false, properties: { reading: { type: "string", enum: numbers } } };
        const input = JSON.stringify({ sentence: q.text, word: w.text + w.tail, readings: Object.fromEntries(readings.map((r, n) => [numbers[n], r])) });
        const { ms, answer } = await evaluate(`(async () => {
          const session = await window.base.clone();
          const t0 = performance.now();
          const answer = await session.prompt(${JSON.stringify(input)}, { responseConstraint: ${JSON.stringify(schema)} });
          const ms = Math.round(performance.now() - t0);
          session.destroy();
          return { ms, answer };
        })()`);
        times.push(ms);
        let picked;
        try {
          picked = numbers.indexOf(JSON.parse(answer).reading);
        } catch {}
        // 候補にない答えは使わない（辞書の読みのまま）
        words.push({ ...w, chosen: picked >= 0 ? w.options[picked] : undefined });
      }
      const chosen = read(words);
      const outcome = right.has(own) ? (right.has(chosen) ? "kept" : "broken") : right.has(chosen) ? "fixed" : "still wrong";
      runs.push({ text: q.text, outcome, dictionary: own, chosen });
      if (outcome !== "kept" && n === 0) console.log(`${outcome}: ${q.text}\n  dictionary ${own}\n  chosen     ${chosen}`);
    }
  }
  close();
  times.sort((a, b) => a - b);
  const count = (o) => runs.filter((r) => r.outcome === o).length;
  const summary = {
    runs: runs.length,
    kept: count("kept"),
    fixed: count("fixed"),
    broken: count("broken"),
    stillWrong: count("still wrong"),
    asked: times.length,
    medianMs: times[Math.floor(times.length / 2)],
    p90Ms: times[Math.floor((times.length * 9) / 10)],
  };
  const out = join(HERE, "results/furigana_nano_choose.json");
  const report = existsSync(out) ? JSON.parse(readFileSync(out, "utf8")) : {};
  report[casesPath.split("/").pop()] = { system: CHOOSE_SYSTEM, summary, runs };
  writeFileSync(out, JSON.stringify(report, null, 1) + "\n");
  console.log("gemini-nano", JSON.stringify(summary));
}

const DECIDE_SYSTEM =
  "You answer one multiple-choice question about a transcript. " +
  'The input is JSON: "state" is the transcript with its context, "question" is what to decide, ' +
  'and "choices" maps each possible answer to its description. ' +
  'Answer with JSON {"choice": ...}: the one key of "choices" that is correct.';

/**
 * Gemini Nano（Prompt API）を、判定モデルと同じ形の口（Jev の `/v1/systemone`）で待たせる（DESIGN.md 9.7）。
 * `split_eval` の terms-jev に `local-…` のモデル名を渡すと、ここに聞きに来る（`local_decider_server.py` と同じポート）。
 * 止めるまで待ち続ける
 */
async function decider(profile) {
  const { evaluate } = await open(profile, []);
  const availability = await evaluate(
    `"LanguageModel" in window ? LanguageModel.availability(${JSON.stringify(LANGUAGES)}) : "no LanguageModel"`,
  );
  console.log("availability:", availability);
  await evaluate(`(async () => {
    window.base = await LanguageModel.create({
      ...${JSON.stringify(LANGUAGES)},
      initialPrompts: [{ role: "system", content: ${JSON.stringify(DECIDE_SYSTEM)} }],
    });
  })()`);
  const times = [];
  const server = createServer(async (req, res) => {
    let body = "";
    for await (const chunk of req) body += chunk;
    const { state, questions } = JSON.parse(body);
    const answers = {};
    // 問いは 1 つずつ聞く（小さいモデルなので、まとめて聞かない）
    for (const [name, q] of Object.entries(questions)) {
      // 答えは選択肢の番号（選択肢の名前を答えの決まりに入れると、Prompt API がエラーを返すことがあった）
      const keys = Object.keys(q.criteria);
      const numbers = keys.map((_, n) => String(n + 1));
      const schema = { type: "object", required: ["choice"], additionalProperties: false, properties: { choice: { type: "string", enum: numbers } } };
      const choices = Object.fromEntries(keys.map((k, n) => [numbers[n], q.criteria[k]]));
      const input = JSON.stringify({ state, question: q.instructions, choices });
      const { ms, answer } = await evaluate(`(async () => {
        const session = await window.base.clone();
        const t0 = performance.now();
        const answer = await session.prompt(${JSON.stringify(input)}, { responseConstraint: ${JSON.stringify(schema)} });
        const ms = Math.round(performance.now() - t0);
        session.destroy();
        return { ms, answer };
      })()`);
      times.push(ms);
      let choice;
      try {
        choice = keys[numbers.indexOf(JSON.parse(answer).choice)];
      } catch {}
      answers[name] = { type: "choice", choice };
    }
    res.setHeader("Content-Type", "application/json");
    res.end(JSON.stringify({ answers, usage: { input_tokens: 0, output_tokens: 0 } }));
    const sorted = [...times].sort((a, b) => a - b);
    console.log(`${times.length} questions, median ${sorted[Math.floor(sorted.length / 2)]} ms per question`);
  });
  await new Promise((r) => server.listen(8765, "127.0.0.1", r));
  console.log("ready");
  await new Promise(() => {});
}

const [mode, ...args] = process.argv.slice(2);
await ({ translate, speech, furigana, choose, decider }[mode])(...args);
process.exit(0);
