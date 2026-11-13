<script lang="ts">
  // モードの名前の横に並べる札。料金（「ベータ期間中は無料」のようなひとことがあれば、その代わりに）と、選ぶ理由。
  // 「モード」のタブの一覧と、「マイクと翻訳」のタブの今のモードで、同じものを出す
  import { Badge } from "$lib/components/ui/badge";
  import { type MODES, monthly, price } from "../../lib/engines";

  let { mode }: { mode: (typeof MODES)[number] } = $props();
</script>

<Badge variant="secondary" class="tabular-nums">
  {mode.badge ?? price(mode.hourlyUsd)}{#if mode.hourlyUsd}（{monthly(mode.hourlyUsd)}）{/if}
</Badge>
{#each mode.traits as trait (trait)}
  <Badge variant="outline">{trait}</Badge>
{/each}
