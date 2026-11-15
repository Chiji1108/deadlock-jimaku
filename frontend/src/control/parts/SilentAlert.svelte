<script lang="ts">
  // 字幕を出しているのに、マイクの音が届いていないときの注意（ホームと「マイクと翻訳」で同じ文を出す）
  import type { Snippet } from "svelte";
  import * as Alert from "$lib/components/ui/alert";
  import { panel } from "../panel.svelte";

  const { children }: { children?: Snippet } = $props();
</script>

{#if panel.silent}
  <Alert.Root variant="destructive">
    <Alert.Title>音が届いていません</Alert.Title>
    <Alert.Description class="flex flex-col items-start gap-2">
      マイクの選択と、OS のマイクの許可（macOS: システム設定 → プライバシーとセキュリティ →
      マイク）を確認してください
      {@render children?.()}
    </Alert.Description>
  </Alert.Root>
{/if}
