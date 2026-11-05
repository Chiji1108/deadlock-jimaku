<script lang="ts">
  // Chrome のモードとハイブリッドモードのページ。アプリが起動し、設定に合わせて聞き始める（DESIGN.md 9.6）
  import { onMount } from "svelte";
  import type { ChromeState } from "../generated/ChromeState";
  import type { Settings } from "../generated/Settings";
  import { api } from "../lib/api";
  import { connect } from "../lib/ws";
  import { Captions } from "./captions";
  import { Live } from "./live";
  import { availability, holdMicrophone, listen, prepare } from "./speech";

  /** 認識を続けられない理由ごとの案内 */
  const FATAL: Record<string, string> = {
    "not-allowed": "マイクを許可してください（アドレスバーの左のアイコンから変えられます）",
    "service-not-allowed": "マイクを許可してください（アドレスバーの左のアイコンから変えられます）",
    "audio-capture": "マイクが見つかりません。つないでから、字幕を始め直してください",
    "language-not-supported": "この言語は Chrome の音声認識に対応していません",
  };
  const UNSUPPORTED = "この Chrome では使えません。Google Chrome を最新にしてください";

  let settings = $state<Settings>();
  let state_ = $state<ChromeState>("idle");
  /** 翻訳モデルのダウンロードにクリックが要る */
  let needsDownload = $state(false);
  let detail = $state("");
  /** 0〜1。ダウンロードしていなければ undefined */
  let progress = $state<number>();
  /** ほかのタブが聞いている */
  let other = $state(false);
  let stop: (() => void) | undefined;
  let cut: (() => void) | undefined;
  /** 同じ設定で聞き直さない。言語や区切りが変わったら聞き直す */
  let listening = "";
  /** 処理は 1 つずつ。途中で設定が変わったら、終わってから合わせ直す */
  let busy = $state(false);
  let changed = false;
  /** 最初の設定を読み終えた（アプリが準備を始めさせられる） */
  let ready = false;

  const key = (s: Settings) =>
    JSON.stringify([s.speechEngine, s.micLanguage, s.translateTo, s.segmentation.minSilenceMs]);
  const wanted = (s: Settings) => s.speechEngine !== "openai" && s.transcriptionEnabled;

  function show(state: ChromeState, text = "") {
    state_ = state;
    detail = text;
    void api.chromeStatus({ state, detail: text || undefined }).catch(() => {});
  }

  function halt() {
    stop?.();
    stop = undefined;
    cut = undefined;
    listening = "";
  }

  /** モードに合わせて字幕の送り先を決める */
  function sink(s: Settings, translators: Awaited<ReturnType<typeof prepare>>) {
    if (s.speechEngine === "hybrid") {
      const live = new Live(translators, api.live);
      return {
        onText: (text: string, final: boolean) => final || live.onText(text),
        onEnd: () => live.cut(),
        discard: () => live.cut(),
        cut: () => live.cut(),
        silenceMs: undefined,
      };
    }
    const captions = new Captions(s.micLanguage, translators, api.segment);
    return {
      onText: (text: string, final: boolean) => captions.onText(text, final),
      onEnd: () => captions.finish(),
      discard: () => captions.discard(),
      cut: () => {},
      silenceMs: s.segmentation.minSilenceMs,
    };
  }

  async function run(s: Settings, targets: string[]) {
    const translators = await prepare(s.micLanguage, targets, (loaded) => {
      progress = loaded;
      state_ = "preparing";
    });
    progress = undefined;
    const target = sink(s, translators);
    const release = await holdMicrophone();
    const recognition = listen(s.micLanguage, {
      silenceMs: target.silenceMs,
      onText: target.onText,
      onEnd: target.onEnd,
      onFatal: (error) => {
        show("failed", FATAL[error] ?? error);
        halt();
      },
    });
    stop = () => {
      recognition.stop();
      release();
      target.discard();
    };
    cut = () => {
      target.cut();
      recognition.cut();
    };
    listening = key(s);
    needsDownload = false;
    show("listening");
  }

  /** 設定に合わせて始める・止める */
  async function apply() {
    const s = (await api.settings()).settings;
    settings = s;
    if (!wanted(s)) {
      halt();
      show("idle");
      return;
    }
    if (listening === key(s)) return;
    halt();
    const { state, targets } = await availability(s.micLanguage, s.translateTo);
    if (state === "unavailable") show("unsupported", UNSUPPORTED);
    else if (state === "available") await run(s, targets);
    else {
      needsDownload = true;
      show("idle");
    }
  }

  async function sync() {
    changed = true;
    if (busy) return;
    busy = true;
    while (changed) {
      changed = false;
      await apply().catch((e: Error) => show("failed", e.message));
    }
    busy = false;
    ready = true;
  }

  /** ダウンロードにはクリックが要る。アプリが代わりに押す。失敗したら手で押せる */
  async function download() {
    if (busy || !needsDownload) return;
    busy = true;
    show("preparing");
    const s = settings!;
    try {
      await run(s, (await availability(s.micLanguage, s.translateTo)).targets);
    } catch (e) {
      show("failed", (e as Error).message);
    }
    busy = false;
    void sync();
  }

  function watch() {
    void sync();
    const disconnect = connect(
      ["status"],
      (msg) => {
        if (msg.type === "settings.changed") void sync();
        if (msg.type === "speech.cut") cut?.();
      },
      // つなぎ直す間に変わった設定に追いつく
      (status) => {
        if (status === "open") void sync();
      },
    );
    (window as unknown as { jimakuStart(): boolean }).jimakuStart = () => {
      if (ready) void download();
      return ready;
    };
    window.addEventListener("pagehide", halt);
    return () => {
      disconnect();
      window.removeEventListener("pagehide", halt);
      halt();
    };
  }

  onMount(() => {
    // 開いたタブが増えても、聞くのは 1 つだけ（字幕が二重に出ない）
    let release = () => {};
    let disconnect = () => {};
    other = true;
    void navigator.locks.request("listening", () => {
      other = false;
      disconnect = watch();
      return new Promise<void>((resolve) => (release = resolve));
    });
    return () => {
      release();
      disconnect();
    };
  });

  const title = $derived.by(() => {
    if (other) return "ほかのウィンドウで動いています";
    if (state_ === "listening") return "字幕を聞き取っています";
    if (state_ === "preparing") return "翻訳の準備をしています";
    if (state_ === "unsupported" || state_ === "failed") return "止まっています";
    return "字幕を待っています";
  });
  const tone = $derived(
    state_ === "listening" ? "on" : state_ === "preparing" ? "wait" : detail ? "bad" : "off",
  );
</script>

<main>
  <p class="brand">デドロ字幕</p>
  <h1><span class="dot {tone}"></span>{title}</h1>

  {#if other}
    <p>このウィンドウは閉じてかまいません。</p>
  {:else if detail}
    <p class="bad-text">{detail}</p>
  {:else if progress !== undefined}
    <progress max="1" value={progress}></progress>
    <p>初回だけ、翻訳のモデルをダウンロードします。</p>
  {:else if state_ === "listening"}
    <p>
      ゲームの後ろに置いたままで大丈夫です。<strong>最小化はしないでください</strong>
      （聞き取りが止まることがあります）。閉じると字幕が止まります。
    </p>
  {:else if settings && settings.speechEngine === "openai"}
    <p>今のモードでは Chrome を使いません。このウィンドウは閉じてかまいません。</p>
  {:else}
    <p>操作パネルで「字幕を始める」を押すと、聞き取りを始めます。</p>
  {/if}

  {#if needsDownload && !busy}
    <button onclick={download}>翻訳のモデルをダウンロードする</button>
  {/if}

  <p class="note">音声は Google の音声認識に送られます。翻訳はこの PC の中で動きます。</p>
</main>

<style>
  :global(body) {
    margin: 0;
    background: #0f1012;
    color: #eceef2;
    font:
      15px/1.7 system-ui,
      "Yu Gothic UI",
      sans-serif;
  }
  main {
    box-sizing: border-box;
    max-width: 460px;
    margin: 0 auto;
    padding: 24px 20px;
  }
  .brand {
    margin: 0 0 12px;
    color: #a0a4ad;
    font-size: 13px;
  }
  h1 {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0 0 8px;
    font-size: 19px;
    font-weight: 600;
  }
  p {
    margin: 0 0 12px;
  }
  .dot {
    flex: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: #6b7078;
  }
  .dot.on {
    background: #34d399;
    animation: pulse 2s ease-in-out infinite;
  }
  .dot.wait {
    background: #38bdf8;
  }
  .dot.bad {
    background: #f87171;
  }
  @keyframes pulse {
    50% {
      opacity: 0.4;
    }
  }
  .bad-text {
    color: #fca5a5;
  }
  progress {
    width: 100%;
    margin-bottom: 8px;
  }
  button {
    margin-bottom: 12px;
    padding: 8px 14px;
    border: 0;
    border-radius: 8px;
    background: #eceef2;
    color: #0f1012;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }
  .note {
    margin: 0;
    color: #a0a4ad;
    font-size: 13px;
  }
</style>
