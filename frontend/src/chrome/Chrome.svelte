<script lang="ts">
  // Chrome のモードの聞き取りのページ。認識した文を core に送る（DESIGN.md 9.6）
  import { onMount } from "svelte";
  import type { ChromeMsg } from "../generated/ChromeMsg";
  import type { LiveStatus } from "../generated/LiveStatus";
  import { api } from "../lib/api";
  import { connect } from "../lib/ws";
  import { holdMicrophone, listen, supported } from "./speech";

  /** 認識を続けられない理由ごとの案内 */
  const FATAL: Record<string, string> = {
    "not-allowed": "マイクを許可してから、このページを読み込み直してください",
    "service-not-allowed": "マイクを許可してから、このページを読み込み直してください",
    "audio-capture": "マイクが見つかりません。つないでから、このページを読み込み直してください",
    "language-not-supported": "この言語は Chrome の音声認識に対応していません",
    network: "音声認識につながりません。ネットワークを確かめて、字幕を始め直してください",
  };

  const usable = supported();
  /** このページの状態。操作パネルにも知らせる */
  let status = $state<LiveStatus>({ state: "preparing" });
  /** 翻訳などの補助プログラムの状態 */
  let helper = $state<LiveStatus>();
  /** ほかのタブが聞いている */
  let other = $state(false);
  let socket: WebSocket | undefined;
  let stop: (() => void) | undefined;
  /** 最新の同期だけが聞き始められる */
  let syncs = 0;

  function send(msg: ChromeMsg) {
    if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify(msg));
  }

  function show(next: LiveStatus) {
    status = next;
    send({ type: "status", status: next });
  }

  function dial() {
    socket = new WebSocket(
      `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/api/chrome/ws`,
    );
    socket.addEventListener("open", () => show(status));
  }

  function halt() {
    stop?.();
    stop = undefined;
  }

  function fail(error: string) {
    halt();
    show({ state: "failed", detail: FATAL[error] ?? error });
  }

  async function start(language: string, silenceMs: number, id: number) {
    let release;
    try {
      release = await holdMicrophone();
    } catch (e) {
      const denied = e instanceof DOMException && e.name === "NotAllowedError";
      fail(denied ? "not-allowed" : "audio-capture");
      return;
    }
    if (id !== syncs) {
      release();
      return;
    }
    const stopListening = listen(language, {
      silenceMs,
      onText: (text, final) => send({ type: "text", text, final }),
      onFatal: fail,
    });
    stop = () => {
      stopListening();
      release();
    };
    show({ state: "listening" });
  }

  /** 字幕を止めているか、翻訳の準備ができていないなら聞かない（音声を Google に送らない） */
  async function sync() {
    const id = ++syncs;
    const { settings } = await api.settings();
    if (id !== syncs) return;
    const wanted =
      settings.speechEngine === "chrome" &&
      settings.transcriptionEnabled &&
      helper?.state === "listening";
    if (!wanted) {
      halt();
      show({ state: "preparing" });
    } else if (!stop) {
      await start(settings.micLanguage, settings.segmentation.minSilenceMs, id);
    }
  }

  function watch() {
    const disconnect = connect(
      ["status"],
      (msg) => {
        if (msg.type !== "live.status") return;
        helper = msg.status ?? undefined;
        void sync().catch((e: Error) => show({ state: "failed", detail: e.message }));
      },
      // アプリが落ちたら聞き続けない。戻ったら、届く状態に合わせて聞き直す
      (connection) => {
        if (connection === "open") return dial();
        syncs++;
        halt();
        socket?.close();
      },
    );
    window.addEventListener("pagehide", halt);
    return () => {
      disconnect();
      window.removeEventListener("pagehide", halt);
      halt();
      socket?.close();
    };
  }

  onMount(() => {
    if (!usable) return;
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
    if (!usable) return "Google Chrome で開いてください";
    if (other) return "ほかのタブで動いています";
    if (status.state === "listening") return "聞き取っています";
    if (status.state === "failed") return "止まっています";
    return "字幕を待っています";
  });
  const tone = $derived(
    !usable || status.state === "failed" ? "bad" : status.state === "listening" ? "on" : "off",
  );
</script>

<main>
  <p class="brand">デドロ字幕</p>
  <h1><span class="dot {tone}"></span>{title}</h1>

  {#if !usable}
    <p>
      音声認識は Google Chrome でだけ使えます。アドレスをコピーして、Google Chrome
      で開いてください。
    </p>
  {:else if other}
    <p>このタブは閉じてかまいません。</p>
  {:else if status.detail}
    <p class="bad-text">{status.detail}</p>
  {:else if status.state === "listening"}
    <p>
      ゲームの後ろに置いたままで大丈夫です。<strong>最小化はしないでください</strong>
      （聞き取りが止まることがあります）。閉じると字幕が止まります。
    </p>
  {:else if helper?.state === "failed"}
    <p class="bad-text">{helper.detail}</p>
  {:else if helper?.state === "preparing"}
    <p>{helper.detail ?? "翻訳の準備をしています"}</p>
  {:else}
    <p>操作パネルで Chrome のモードの字幕を始めると、聞き取りを始めます。</p>
  {/if}

  <p class="note">音声は Chrome を通じて Google に送られます。翻訳はこの PC の中で動きます。</p>
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
  .note {
    margin: 0;
    color: #a0a4ad;
    font-size: 13px;
  }
</style>
