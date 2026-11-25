<script lang="ts">
  // 字幕に使うマイクが Bluetooth のときの注意（ホームと「マイクと翻訳」で同じ文を出す）。
  // Bluetooth のイヤホンは、マイクを開くと通話用の接続に替わり、聞こえる音が悪くなる
  import type { Snippet } from "svelte";
  import * as Alert from "$lib/components/ui/alert";
  import { panel } from "../panel.svelte";

  const { children }: { children?: Snippet } = $props();
</script>

{#if panel.mic?.bluetooth}
  <Alert.Root>
    <Alert.Title>Bluetooth のマイクを使います</Alert.Title>
    <Alert.Description class="flex flex-col items-start gap-2">
      字幕を出している間、このイヤホンやヘッドホンから聞こえる音が悪くなります（Bluetooth
      は、マイクを使うと通話用の音に切り替わります）。ほかのマイクを選ぶと、音は悪くなりません
      {@render children?.()}
    </Alert.Description>
  </Alert.Root>
{/if}
