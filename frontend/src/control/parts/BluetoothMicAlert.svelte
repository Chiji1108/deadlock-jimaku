<script lang="ts">
  // 字幕に使うマイクが Bluetooth のときの警告（ホームと「マイクと翻訳」で同じ文を出す）。
  // Bluetooth のイヤホンは、マイクを開くと通話用の接続に替わり、聞こえる音が悪くなる。
  // 配信者がわざわざ選ぶマイクではないので、既定のマイクがそうなっている間違いとして強く出す
  import type { Snippet } from "svelte";
  import TriangleAlertIcon from "@lucide/svelte/icons/triangle-alert";
  import * as Alert from "$lib/components/ui/alert";
  import { panel } from "../panel.svelte";

  const { children }: { children?: Snippet } = $props();
</script>

{#if panel.mic?.bluetooth}
  <Alert.Root variant="destructive">
    <TriangleAlertIcon />
    <Alert.Title>マイクが Bluetooth のイヤホンになっています</Alert.Title>
    <Alert.Description class="flex flex-col items-start gap-2">
      このまま字幕を始めると、イヤホンやヘッドホンから聞こえる音が悪くなります（Bluetooth
      は、マイクを使うと通話用の音に切り替わります）。配信で使っているマイクを選んでください
      {@render children?.()}
    </Alert.Description>
  </Alert.Root>
{/if}
