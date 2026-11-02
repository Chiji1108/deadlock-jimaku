<script lang="ts">
  import { onMount } from "svelte";
  import { connect, type Status } from "../lib/ws";

  let status = $state<Status>("connecting");
  let copied = $state(false);

  const overlayUrl = `${location.origin}/overlay/subtitles`;

  onMount(() =>
    connect(
      ["subtitle"],
      () => {},
      (s) => (status = s),
    ),
  );

  async function copy() {
    await navigator.clipboard.writeText(overlayUrl);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }

  async function demo() {
    await fetch("/api/demo", { method: "POST" });
  }
</script>

<main>
  <h1>Deadlock Stream Overlays</h1>
  <p class="status {status}">
    サーバー: {status === "open" ? "接続中" : status === "connecting" ? "接続しています…" : "切断"}
  </p>

  <section>
    <h2>字幕オーバーレイ</h2>
    <p>OBS のブラウザソースに次の URL を設定してください。</p>
    <div class="url">
      <code>{overlayUrl}</code>
      <button onclick={copy}>{copied ? "コピーしました" : "コピー"}</button>
    </div>
    <button onclick={demo} disabled={status !== "open"}>テスト字幕を流す</button>
  </section>
</main>

<style>
  :global(body) {
    margin: 0;
    font-family: system-ui, sans-serif;
    background: #16181d;
    color: #e8e8ea;
  }
  main {
    max-width: 720px;
    margin: 0 auto;
    padding: 24px 16px;
  }
  h1 {
    font-size: 20px;
  }
  h2 {
    font-size: 16px;
    margin-top: 28px;
  }
  .status.open {
    color: #6fdc8c;
  }
  .status.closed {
    color: #ff7b72;
  }
  .url {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-bottom: 16px;
  }
  code {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
    padding: 6px 8px;
    background: #22252c;
    border-radius: 6px;
  }
  button {
    padding: 6px 14px;
    border: 1px solid #3a3f4b;
    border-radius: 6px;
    background: #2b2f38;
    color: inherit;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
