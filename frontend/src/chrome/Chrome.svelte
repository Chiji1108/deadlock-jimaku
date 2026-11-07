<script lang="ts">
  // Chrome のモードのページ。アプリが起動し、認識した文を core に送り、頼まれた文を訳す（DESIGN.md 9.6）
  import { onMount } from "svelte";
  import type { ChromeMsg } from "../generated/ChromeMsg";
  import type { ChromeCommand } from "../generated/ChromeCommand";
  import type { LiveStatus } from "../generated/LiveStatus";
  import type { Settings } from "../generated/Settings";
  import { api } from "../lib/api";
  import { connect } from "../lib/ws";
  import {
    availability,
    chosenMic,
    DEFAULT_MIC,
    listen,
    micNotice,
    micUnanswered,
    openMicrophone,
    prepare,
    rememberMic,
    supported,
    type MicChoice,
    type MicTrouble,
  } from "./speech";
  import { Translation } from "./translation";

  /** ブロックしたあとは確認が出ない。OS で止めていても同じエラーになる */
  const MIC_DENIED =
    "マイクを使えません。Chrome の確認でブロックしたなら、操作パネルの「マイクと翻訳」にある「Chrome をリセット」を押すと、もう一度確認が出ます。許可しているなら、OS の設定で Google Chrome のマイクを許可してください";
  /** 認識を続けられない理由ごとの案内 */
  const FATAL: Record<string, string> = {
    "not-allowed": MIC_DENIED,
    "service-not-allowed": MIC_DENIED,
    "audio-capture": "マイクが見つかりません。つないでから、字幕を始め直してください",
    "language-not-supported": "この言語は Chrome の音声認識に対応していません",
  };
  /** core が閉じた理由: 新しいページに聞く役を渡した（crates/core/src/server.rs と同じ値） */
  const REPLACED = 4001;
  /** 翻訳先が複数あると、2 つ目以降はやり直しが要る */
  const DOWNLOAD_ATTEMPTS = 3;
  /** `/api/chrome/ws` だけ切れたときに、つなぎ直すまでの時間 */
  const REDIAL_MS = 1000;
  /** アプリが落ちても Chrome を残さない */
  const ORPHAN_MS = 5000;
  const UNSUPPORTED = "この Chrome では使えません。Google Chrome を最新にしてください";
  /** 続いているエラーの案内。聞き直しは続けている */
  const TROUBLE: Record<string, string> = {
    network: "音声認識につながりません。回線が戻ると自動で再開します",
    "audio-capture": "マイクの音を取れません。つなぎ直すと自動で再開します",
  };
  const names = new Intl.DisplayNames(["ja"], { type: "language" });

  const usable = supported();
  /** このページの状態。操作パネルにも知らせる */
  let status = $state<LiveStatus>({ state: "preparing" });
  /** 字幕の処理が文を受け取れるか */
  let helper = $state<LiveStatus>();
  /** ほかのページが聞いている */
  let other = $state(false);
  /** 翻訳モデルのダウンロードにクリックが要る */
  let needsDownload = $state(false);
  let settings: Settings | undefined;
  let socket: WebSocket | undefined;
  let stop: (() => void) | undefined;
  /** ハイブリッドの発話が閉じた */
  let cut: (() => void) | undefined;
  /** ハイブリッドの発話が開いた */
  let reset: (() => void) | undefined;
  /** 今の区切りの番号。core が前の発話の文を見分ける */
  let cuts = 0;
  /** `/ws` がつながっている。切れている間はアプリがないので、`/api/chrome/ws` もつなぎ直さない */
  let online = false;
  let redial: ReturnType<typeof setTimeout>;
  /** `/ws` の購読をやめる */
  let disconnect = () => {};
  /** 同じ設定で聞き直さない。言語や区切りが変わったら聞き直す */
  let listening = "";
  /** 処理は 1 つずつ。途中で設定が変わったら、終わってから合わせ直す */
  let busy = $state(false);
  let changed = false;
  /** 最初の同期を終えた（アプリが準備を始めさせられる） */
  let ready = false;
  /** ダウンロードに続けて失敗した回数。上限のあとは手で押してもらう */
  let failures = 0;
  /** 訳せない言語の案内。聞いている間ずっと出す */
  let notice: string | undefined;
  /** 選んだマイクを使えない案内 */
  let micTrouble: string | undefined;
  /** 自動で見つからないときに選べるマイク。空なら選択欄を出さない */
  let micChoices = $state<MicChoice[]>([]);
  /** 選択欄のマイクの ID。空なら Chrome の既定のマイク。開き直したら、実際に開いたマイクに揃える */
  let micChosen = $state("");
  /** 選択欄を押した回数。同じマイクを選び直しても開き直す */
  let micPicks = 0;
  /** ハイブリッドなら、選んだマイクはアプリのマイクとずれることがある */
  let hybrid = $state(false);
  /** マイクを調べるための記録。機器の名前が入るので、外に送らず、押されたらコピーするだけ */
  let micLog = $state<string[]>([]);
  let copyResult = $state<string>();
  /** 止めるたびに増える。待っている間に止められた処理が、あとから聞き始めないようにする */
  let runs = 0;
  let translation = new Translation(new Map(), reply);
  /** 今の `translation` の言語。マイクだけ変えるなら作り直さない */
  let translatorLanguages = "";

  function send(msg: ChromeMsg) {
    if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify(msg));
  }

  function reply(translated: Omit<Extract<ChromeMsg, { type: "translated" }>, "type">) {
    send({ type: "translated", ...translated });
  }

  function show(next: LiveStatus) {
    status = next;
    send({ type: "status", status: next });
  }

  const showListening = () =>
    show({
      state: "listening",
      detail: [micTrouble, notice].filter(Boolean).join("。") || undefined,
    });

  function dial() {
    const next = new WebSocket(
      `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/api/chrome/ws`,
    );
    socket = next;
    next.addEventListener("open", () => {
      show(status);
      void sync();
    });
    next.addEventListener("message", (e) => {
      const command = JSON.parse(e.data as string) as ChromeCommand;
      if (command.type === "translate") return translation.onTask(command);
      if (command.type === "open") return reset?.();
      cuts = command.count;
      cut?.();
    });
    next.addEventListener("close", ({ code }) => {
      if (code === REPLACED) {
        other = true;
        halt();
        disconnect();
      } else if (online && next === socket) redial = setTimeout(dial, REDIAL_MS);
    });
  }

  function halt() {
    runs++;
    stop?.();
    stop = undefined;
    cut = undefined;
    reset = undefined;
    listening = "";
  }

  function fail(error: string) {
    halt();
    needsDownload = false;
    show({ state: "failed", detail: FATAL[error] ?? error });
  }

  const key = (s: Settings) =>
    JSON.stringify([
      s.speechEngine,
      s.micDevice,
      s.micLanguage,
      s.translateTo,
      s.segmentation.minSilenceMs,
      micPicks,
    ]);
  /** 字幕を止めているか、core の準備ができていないなら聞かない（音声を Google に送らない） */
  const wanted = (s: Settings) =>
    s.speechEngine !== "openai" &&
    s.transcriptionEnabled &&
    helper?.state === "listening" &&
    socket?.readyState === WebSocket.OPEN;

  /** ハイブリッドの発話はアプリが区切るので、Chrome が途中で確定した文をつなげる。
   * 英語などは単語の前の空白も認識の結果に入っている */
  function hybridSink() {
    let settled = "";
    return {
      silenceMs: undefined,
      reset: () => (settled = ""),
      onText(text: string, final: boolean) {
        const joined = `${settled}${text}`;
        if (final) settled = joined;
        if (joined.trim()) send({ type: "text", text: joined.trim(), final: false, cut: cuts });
      },
    };
  }

  /** Chrome のモードの発話は、Chrome の確定と文の区切りの時間で分かれる */
  function chromeSink(s: Settings) {
    return {
      silenceMs: s.segmentation.minSilenceMs,
      reset: () => {},
      onText: (text: string, final: boolean) =>
        send({ type: "text", text: text.trim(), final, cut: cuts }),
    };
  }

  function chooseMic(deviceId: string) {
    const mic = micChoices.find((d) => d.deviceId === deviceId) ?? DEFAULT_MIC;
    try {
      rememberMic(mic, settings?.micDevice);
    } catch (e) {
      logMic(`マイクの選択を覚えられません: ${(e as Error).name}`);
    }
    micPicks++;
    void sync();
  }

  async function open(s: Settings) {
    try {
      if (await micUnanswered()) {
        show({
          state: "preparing",
          detail: "Chrome のウィンドウで「サイトへのアクセス時は許可」を押してください",
        });
      }
      return await openMicrophone(s.micDevice, logMic, chosenMic(s.micDevice));
    } catch (e) {
      const denied = e instanceof DOMException && e.name === "NotAllowedError";
      fail(denied ? "not-allowed" : "audio-capture");
    }
  }

  const logMic = (line: string) => micLog.push(line);

  async function run(s: Settings, targets: string[]) {
    const started = runs;
    // 始めたあとに届いた変更は、ここの値と比べて分かる
    const running = key(s);
    const languages = JSON.stringify([s.micLanguage, targets]);
    if (languages !== translatorLanguages) {
      const translators = await prepare(s.micLanguage, targets, (loaded) =>
        show({
          state: "preparing",
          detail: `翻訳のモデルをダウンロードしています（${Math.round(loaded * 100)}%）`,
        }),
      );
      translation = new Translation(translators, reply);
      translatorLanguages = languages;
    }
    const missing = s.translateTo.filter((target) => !targets.includes(target));
    notice = missing.length
      ? `Chrome の翻訳が対応していない言語は訳しません: ${missing.map((l) => names.of(l)).join("、")}`
      : undefined;
    // 待っている間に止められたら、聞き始めない
    if (started !== runs) return;
    micLog = [navigator.userAgent, `アプリのマイク: ${s.micDevice ?? "OS の既定"}`];
    copyResult = undefined;
    const microphone = await open(s);
    if (!microphone) return;
    if (started !== runs) {
      microphone.release();
      return;
    }
    const showMic = (trouble?: MicTrouble, using?: MicChoice) => {
      micTrouble = trouble && micNotice(trouble, using?.label);
      if (trouble && microphone.choices.length) micTrouble += "。Chrome のウィンドウで選べます";
      micChoices = trouble || microphone.chosen ? microphone.choices : [];
      micChosen = using?.deviceId ?? DEFAULT_MIC.deviceId;
    };
    showMic(microphone.trouble, microphone.opened);
    hybrid = s.speechEngine === "hybrid";
    const sink = s.speechEngine === "hybrid" ? hybridSink() : chromeSink(s);
    const recognition = listen(s.micLanguage, {
      silenceMs: sink.silenceMs,
      onText: sink.onText,
      onFatal: fail,
      onTrouble: (error) =>
        error
          ? show({
              state: "blocked",
              detail: TROUBLE[error] ?? `音声認識のエラーが続いています（${error}）`,
            })
          : showListening(),
      track: microphone.track,
      heardSound: microphone.heardSound,
      onDefaultMic(problem) {
        showMic({ name: microphone.opened?.label ?? s.micDevice ?? "", problem });
        showListening();
        microphone.useDefault().catch((e: Error) => logMic(`既定のマイクを開けません: ${e.name}`));
      },
      onMicLog: logMic,
    });
    stop = () => {
      recognition.stop();
      microphone.release();
      sink.reset();
    };
    reset = sink.reset;
    cut = () => {
      sink.reset();
      recognition.cut();
    };
    listening = running;
    needsDownload = false;
    showListening();
  }

  /** 設定に合わせて始める・止める */
  async function apply() {
    const s = (await api.settings()).settings;
    settings = s;
    if (!wanted(s)) {
      halt();
      needsDownload = false;
      show({ state: "preparing" });
      return;
    }
    if (listening === key(s)) return;
    halt();
    const started = runs;
    const { state, targets } = await availability(s.micLanguage, s.translateTo);
    if (started !== runs) return;
    if (state === "unavailable") show({ state: "failed", detail: UNSUPPORTED });
    else if (state === "available") await run(s, targets);
    else {
      needsDownload = true;
      if (failures === 0) show({ state: "preparing", detail: "翻訳のモデルを準備しています" });
    }
  }

  async function sync() {
    changed = true;
    if (busy) return;
    busy = true;
    while (changed) {
      changed = false;
      await apply().catch((e: Error) => show({ state: "failed", detail: e.message }));
    }
    busy = false;
    ready = true;
  }

  /** ダウンロードにはクリックが要る。アプリが代わりに押す。失敗したら手で押せる */
  async function download() {
    if (busy || !needsDownload || !settings) return;
    busy = true;
    const s = settings;
    try {
      const started = runs;
      const { targets } = await availability(s.micLanguage, s.translateTo);
      if (started === runs) await run(s, targets);
      failures = 0;
    } catch (e) {
      failures++;
      show({ state: "failed", detail: (e as Error).message });
    }
    busy = false;
    void sync();
  }

  onMount(() => {
    if (!usable) return;
    let orphan: ReturnType<typeof setTimeout>;
    disconnect = connect(
      ["status"],
      (msg) => {
        if (msg.type !== "live.status") return;
        helper = msg.status ?? undefined;
        void sync();
      },
      // アプリが落ちたら聞き続けない。戻ったら、届く状態に合わせて聞き直す
      (connection) => {
        clearTimeout(orphan);
        clearTimeout(redial);
        if (other) return;
        online = connection === "open";
        if (online) return dial();
        halt();
        socket?.close();
        orphan = setTimeout(() => window.close(), ORPHAN_MS);
      },
    );
    (window as unknown as { jimakuStart(): boolean }).jimakuStart = () => {
      if (ready && failures < DOWNLOAD_ATTEMPTS) void download();
      return ready;
    };
    window.addEventListener("pagehide", halt);
    return () => {
      disconnect();
      clearTimeout(orphan);
      clearTimeout(redial);
      online = false;
      window.removeEventListener("pagehide", halt);
      halt();
      socket?.close();
    };
  });

  async function copyMicLog() {
    copyResult = await navigator.clipboard
      .writeText(micLog.join("\n"))
      .then(() => "コピーしました")
      .catch((e: Error) => `コピーできません（${e.name}）`);
  }

  const title = $derived.by(() => {
    if (!usable) return "Google Chrome で開いてください";
    if (other) return "ほかのウィンドウで動いています";
    if (status.state === "listening") return "聞き取っています";
    if (status.state === "failed") return "止まっています";
    if (status.state === "blocked") return "待っています";
    return "字幕を待っています";
  });
  const tone = $derived.by(() => {
    if (!usable || status.state === "failed" || status.state === "blocked") return "bad";
    return status.state === "listening" ? "on" : "off";
  });
</script>

<main>
  <p class="brand">デドロ字幕</p>
  <h1><span class="dot {tone}"></span>{title}</h1>

  {#if !usable}
    <p>
      音声認識は Google Chrome でだけ使えます。操作パネルで字幕を始めると、アプリが Google Chrome
      の小さなウィンドウを開きます。
    </p>
  {:else if other}
    <p>このウィンドウは閉じてかまいません。</p>
  {:else if status.detail}
    <p class:bad-text={tone === "bad"}>{status.detail}</p>
  {:else if status.state === "listening"}
    <p>
      ゲームの後ろに置いたままで大丈夫です。<strong>最小化はしないでください</strong>
      （聞き取りが止まることがあります）。閉じると字幕が止まります。
    </p>
  {:else}
    <p>操作パネルで字幕を始めると、聞き取りを始めます。</p>
  {/if}

  {#if needsDownload && !busy}
    <button onclick={() => ((failures = 0), download())}>翻訳のモデルをダウンロードする</button>
  {/if}

  {#if micChoices.length && (status.state === "listening" || status.state === "blocked")}
    <label class="mic">
      認識に使うマイク
      <select bind:value={micChosen} onchange={(e) => chooseMic(e.currentTarget.value)}>
        <option value={DEFAULT_MIC.deviceId}>Chrome の既定のマイク</option>
        {#each micChoices as mic (mic.deviceId)}
          <option value={mic.deviceId}>{mic.label}</option>
        {/each}
      </select>
    </label>
    {#if hybrid}
      <p class="note">アプリのマイクと違うものを選ぶと、途中の字幕と確定の字幕がずれます。</p>
    {/if}
  {/if}

  <p class="note">音声は Chrome を通じて Google に送られます。翻訳はこの PC の中で動きます。</p>

  {#if micLog.length}
    <button class="quiet" onclick={copyMicLog}>
      {copyResult ?? "マイクの記録をコピー"}
    </button>
  {/if}
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
  .mic {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: 12px;
    font-size: 13px;
  }
  select {
    padding: 6px 8px;
    border: 1px solid #3a3d44;
    border-radius: 8px;
    background: #1a1c20;
    color: inherit;
    font: inherit;
  }
  .quiet {
    margin: 8px 0 0;
    padding: 0;
    background: none;
    color: #a0a4ad;
    font-size: 13px;
    font-weight: 400;
    text-decoration: underline;
  }
  .note {
    margin: 0;
    color: #a0a4ad;
    font-size: 13px;
  }
</style>
