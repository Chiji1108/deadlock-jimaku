<script lang="ts">
  // 無料モードのページ。設定が「字幕を始める」になったら聞き始め、字幕をアプリへ送る（DESIGN.md 9.6）
  import { onMount } from "svelte";
  import type { Settings } from "../generated/Settings";
  import { api } from "../lib/api";
  import { connect } from "../lib/ws";
  import { Captions } from "./captions";
  import { availability, listen, prepare } from "./speech";

  /** 認識を止められない理由ごとの案内 */
  const FATAL: Record<string, string> = {
    "not-allowed": "マイクを許可してください（アドレスバーの左のアイコンから変えられます）",
    "service-not-allowed": "マイクを許可してください（アドレスバーの左のアイコンから変えられます）",
    "audio-capture": "マイクが見つかりません。つないでから、字幕を始め直してください",
    "language-not-supported": "この言語は Chrome の音声認識に対応していません",
  };

  let settings = $state<Settings>();
  let state_ = $state<"idle" | "download" | "preparing" | "running" | "unsupported">("idle");
  let message = $state("");
  let stop: (() => void) | undefined;
  /** 同じ設定で聞き直さない。言語が変わったら聞き直す */
  let listening = "";
  /** 処理は 1 つずつ。途中で設定が変わったら、終わってから合わせ直す */
  let busy = false;
  let changed = false;

  const key = (s: Settings) => JSON.stringify([s.micLanguage, s.translateTo]);
  const wanted = (s?: Settings) => s?.speechEngine === "chrome" && s.transcriptionEnabled;

  function halt() {
    stop?.();
    stop = undefined;
    listening = "";
  }

  async function run(s: Settings, targets: string[]) {
    const translators = await prepare(s.micLanguage, targets);
    const captions = new Captions(s.micLanguage, translators, api.segment);
    const end = listen(s.micLanguage, {
      onText: (text, final) => captions.onText(text, final),
      onEnd: () => captions.discard(),
      onFatal: (error) => {
        message = FATAL[error] ?? error;
        halt();
        void api.updateSettings({ transcriptionEnabled: false }).catch(() => {});
      },
    });
    stop = () => {
      end();
      captions.discard();
    };
    listening = key(s);
    state_ = "running";
    message = "";
  }

  /** 設定に合わせて始める・止める */
  async function apply() {
    const s = (await api.settings()).settings;
    settings = s;
    if (!wanted(s)) {
      halt();
      state_ = "idle";
      return;
    }
    if (listening === key(s)) return;
    halt();
    state_ = "idle";
    const { state, targets } = await availability(s.micLanguage, s.translateTo);
    if (state === "unavailable") state_ = "unsupported";
    else if (state === "available") await run(s, targets);
    else state_ = "download";
  }

  async function sync() {
    changed = true;
    if (busy) return;
    busy = true;
    while (changed) {
      changed = false;
      await apply().catch((e: Error) => (message = e.message));
    }
    busy = false;
  }

  /** ダウンロードにはクリックが要る */
  async function download() {
    if (busy) return;
    busy = true;
    state_ = "preparing";
    const s = settings!;
    try {
      await run(s, (await availability(s.micLanguage, s.translateTo)).targets);
    } catch (e) {
      message = `${(e as Error).message}（もう一度押してください）`;
      state_ = "download";
    }
    busy = false;
    void sync();
  }

  onMount(() => {
    void sync();
    const disconnect = connect(
      ["status"],
      (msg) => {
        if (msg.type === "settings.changed") void sync();
      },
      // つなぎ直す間に変わった設定に追いつく
      (status) => {
        if (status === "open") void sync();
      },
    );
    window.addEventListener("pagehide", halt);
    return () => {
      disconnect();
      window.removeEventListener("pagehide", halt);
      halt();
    };
  });
</script>

<main>
  <h1>デドロ字幕（無料モード）</h1>
  {#if settings && settings.speechEngine !== "chrome"}
    <p>操作パネルの「マイクと翻訳」で、音声認識を「Chrome（無料）」にしてください。</p>
  {:else if state_ === "unsupported"}
    <p>
      このブラウザーでは使えません。Google Chrome でこのページを開いてください。Chrome
      が古いときは、更新してください。
    </p>
  {:else if state_ === "download"}
    <p>音声認識と翻訳のモデルをダウンロードします（初回だけ）。</p>
    <button onclick={download}>ダウンロードして始める</button>
  {:else if state_ === "preparing"}
    <p>ダウンロードしています…</p>
  {:else if state_ === "running"}
    <p class="running">字幕を出しています。このタブは閉じないでください。</p>
  {:else}
    <p>操作パネルの「字幕を始める」を押すと、ここで音声認識が始まります。</p>
  {/if}
  {#if message}<p class="error">{message}</p>{/if}
  <p class="note">音声は PC の外に送られません。</p>
</main>

<style>
  :global(body) {
    margin: 0;
    background: #16171a;
    color: #eceef2;
    font:
      16px/1.7 system-ui,
      "Yu Gothic UI",
      sans-serif;
  }
  main {
    max-width: 560px;
    margin: 0 auto;
    padding: 48px 16px;
  }
  h1 {
    font-size: 20px;
    margin: 0 0 16px;
  }
  button {
    font: inherit;
    padding: 8px 16px;
  }
  .running {
    color: #6ee7a8;
  }
  .error {
    color: #ff8a80;
  }
  .note {
    color: #a0a4ad;
    font-size: 14px;
  }
</style>
