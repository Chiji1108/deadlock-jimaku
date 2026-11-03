<script lang="ts">
  // オーバーレイの中身をソースの箱のどこに寄せるか、3×3 のボタンから選ぶ
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import type { Anchor } from "../../generated/Anchor";
  import { ANCHORS } from "../../lib/anchor";

  let {
    value,
    label,
    onchange,
  }: { value: Anchor; label: string; onchange: (anchor: Anchor) => void } = $props();
</script>

<ToggleGroup.Root
  type="single"
  variant="outline"
  spacing={1}
  aria-label={label}
  class="grid grid-cols-[repeat(3,--spacing(9))]"
  {value}
  onValueChange={(v) => v && onchange(v as Anchor)}
>
  {#each ANCHORS as a (a.value)}
    <ToggleGroup.Item value={a.value} aria-label={a.label} title={a.label} class="size-9">
      <span class="size-2.5 rounded-sm bg-current {a.value === value ? '' : 'opacity-40'}"></span>
    </ToggleGroup.Item>
  {/each}
</ToggleGroup.Root>
