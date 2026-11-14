<script lang="ts">
  // 字幕に使う Chrome のウィンドウの今の状態と、開き直す・リセットする操作。途中の字幕を Chrome が作るときだけ出す
  import * as Alert from "$lib/components/ui/alert";
  import { Button } from "$lib/components/ui/button";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import { panel } from "../panel.svelte";
  import { anyPaid, chromeFinals } from "../../lib/engines";

  const s = $derived(panel.settings!);
  const t = $derived(s.engines);
  /** Chrome のウィンドウが字幕を止めている理由。止まっているか、回線の回復を待っている */
  const problem = $derived(
    panel.chromeStatus?.state === "failed" ||
      panel.chromeStatus?.state === "blocked" ||
      panel.chromeStatus?.state === "missing"
      ? panel.chromeStatus.detail
      : undefined,
  );
  /** 鍵の要る工程があるか、確定をアプリが作る字幕が止まったら Chrome を閉じる（字幕にならない音声を Google に
   * 送らない）。PC の中の文字起こしを準備している間も閉じる。core の `chrome_needed` と同じ決まり */
  const closed = $derived(
    ((anyPaid(t) || !chromeFinals(t)) && !!panel.transcriptionError) || panel.preparing,
  );
  const note = $derived.by(() => {
    if (closed) return "字幕が止まっているので閉じています";
    if (!panel.chromeStatus) return "Chrome のウィンドウを開いています";
    if (panel.chromeStatus.detail) return panel.chromeStatus.detail;
    if (panel.chromeStatus.state === "listening") return "聞き取っています";
    return "字幕の準備をしています";
  });
  let confirmingReset = $state(false);
</script>

{#snippet resetButton()}
  <Button variant="outline" size="sm" onclick={() => (confirmingReset = true)}>
    Chrome をリセット
  </Button>
{/snippet}

{#if s.transcriptionEnabled && problem}
  <Alert.Root variant="destructive">
    <Alert.Title>{problem}</Alert.Title>
    <Alert.Description class="flex flex-col items-start gap-2">
      {#if !chromeFinals(t)}
        確定の字幕は出ています。話している途中の字幕だけが止まっています。
      {/if}
      {#if panel.chromeStatus?.state === "missing"}
        <Button
          variant="outline"
          size="sm"
          href="https://www.google.com/chrome/"
          target="_blank"
          rel="noreferrer"
        >
          Google Chrome をインストールする
        </Button>
      {:else}
        {@render resetButton()}
      {/if}
    </Alert.Description>
  </Alert.Root>
{:else}
  <div class="flex flex-wrap items-center gap-x-3 gap-y-2">
    <p class="min-w-40 flex-1 text-xs text-muted-foreground">
      {#if s.transcriptionEnabled}
        Chrome: {note}
      {:else}
        字幕を始めると小さな Chrome のウィンドウが開きます。音声は Chrome を通じて Google
        に送られます。字幕を出す間はウィンドウを閉じないでください
      {/if}
    </p>
    {#if s.transcriptionEnabled && !closed}
      <Button variant="outline" size="sm" onclick={panel.openChrome}>Chrome を開き直す</Button>
    {/if}
    {@render resetButton()}
  </div>
{/if}

<ConfirmDialog
  bind:open={confirmingReset}
  title="Chrome をリセットしますか？"
  action="リセットする"
  destructive
  onconfirm={panel.resetChrome}
>
  <p>
    字幕に使う Chrome のウィンドウを閉じて、専用のデータを消します。ふだん使っている Chrome
    には影響しません。
  </p>
  <p>
    マイクの許可はもう一度聞かれ、ウィンドウで選んだマイクは戻ります。翻訳のモデルもダウンロードし直すことがあります。
  </p>
  {#if s.transcriptionEnabled}<p>字幕を出している間なら、Chrome は自動で起動し直します。</p>{/if}
</ConfirmDialog>
