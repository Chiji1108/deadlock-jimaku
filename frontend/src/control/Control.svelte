<script lang="ts">
  import { onMount } from "svelte";
  import { connect, type Status } from "../lib/ws";

  let status = $state<Status>("connecting");
  let copied = $state<string | undefined>();

  const overlays = {
    subtitles: `${location.origin}/overlay/subtitles`,
    rank: `${location.origin}/overlay/rank`,
  };

  onMount(() =>
    connect(
      ["subtitle"],
      () => {},
      (s) => (status = s),
    ),
  );

  async function copy(url: string) {
    await navigator.clipboard.writeText(url);
    copied = url;
    setTimeout(() => (copied = undefined), 1500);
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

  {#snippet url(href: string)}
    <div class="url">
      <code>{href}</code>
      <button onclick={() => copy(href)}>{copied === href ? "コピーしました" : "コピー"}</button>
    </div>
  {/snippet}

  <section>
    <h2>字幕オーバーレイ</h2>
    <p>OBS のブラウザソースに次の URL を設定してください。</p>
    {@render url(overlays.subtitles)}
    <button onclick={demo} disabled={status !== "open"}>テスト字幕を流す</button>
  </section>

  <section>
    <h2>ランクオーバーレイ</h2>
    <p>ランク名の言語は <code>?lang=en</code> などで変えられます。</p>
    {@render url(overlays.rank)}
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
