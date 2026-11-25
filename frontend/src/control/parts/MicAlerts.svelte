<script lang="ts">
  // マイクの警告。どのタブでも、いちばん上（Control.svelte の警告の並び）に出す。
  // 直す場所は「マイクと翻訳」なので、ほかのタブではそこへ送るボタンを付ける
  import TriangleAlertIcon from "@lucide/svelte/icons/triangle-alert";
  import * as Alert from "$lib/components/ui/alert";
  import { Button } from "$lib/components/ui/button";
  import { panel } from "../panel.svelte";
</script>

{#snippet choose()}
  {#if panel.tab !== "audio"}
    <Button variant="outline" size="sm" onclick={() => (panel.tab = "audio")}>マイクを選ぶ</Button>
  {/if}
{/snippet}

<!-- Bluetooth のイヤホンは、マイクを開くと通話用の接続に替わり、聞こえる音が悪くなる。
  配信者がわざわざ選ぶマイクではないので、既定のマイクがそうなっている間違いとして強く出す -->
{#if panel.bluetoothMic}
  <Alert.Root variant="destructive">
    <TriangleAlertIcon />
    <Alert.Title>マイクが Bluetooth のイヤホンになっています</Alert.Title>
    <Alert.Description class="flex flex-col items-start gap-2">
      このまま字幕を始めると、イヤホンやヘッドホンから聞こえる音が悪くなります（Bluetooth
      は、マイクを使うと通話用の音に切り替わります）。配信で使っているマイクを選んでください
      {@render choose()}
    </Alert.Description>
  </Alert.Root>
{/if}

<!-- Chrome が、アプリのマイクを使えずに Chrome の既定のマイクへ戻った。違うマイクの声で字幕になる。
  Chrome のウィンドウで選んだマイクは、アプリのマイクと同じものとして扱う（ここには来ない） -->
{#if panel.chromeMissed}
  <Alert.Root variant="destructive">
    <TriangleAlertIcon />
    <Alert.Title>Chrome が別のマイクを使っています</Alert.Title>
    <Alert.Description>
      Chrome が『{panel.chromeMissed}』を使えず、『{panel.chromeMic
        ?.label}』で聞き取っています。字幕に使う Chrome のウィンドウでマイクを選ぶか、OS
      の既定のマイクを『{panel.chromeMissed}』にしてください
    </Alert.Description>
  </Alert.Root>
{/if}

<!-- 字幕を始めてから、マイクの音が一度も届いていない -->
{#if panel.silent}
  <Alert.Root variant="destructive">
    <TriangleAlertIcon />
    <Alert.Title>音が届いていません</Alert.Title>
    <Alert.Description class="flex flex-col items-start gap-2">
      マイクの選択と、OS のマイクの許可（macOS: システム設定 → プライバシーとセキュリティ →
      マイク）を確認してください
      {@render choose()}
    </Alert.Description>
  </Alert.Root>
{/if}
