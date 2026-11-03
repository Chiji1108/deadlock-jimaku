<script lang="ts">
  // オーバーレイの中身をソースの箱のどこに寄せるか、3×3 のボタンから選ぶ
  import type { Anchor } from "../../generated/Anchor";
  import { ANCHORS } from "../../lib/anchor";

  let {
    value,
    label,
    onchange,
  }: { value: Anchor; label: string; onchange: (anchor: Anchor) => void } = $props();
</script>

<div role="radiogroup" aria-label={label} class="grid w-fit grid-cols-3 gap-1">
  {#each ANCHORS as a (a.value)}
    <button
      type="button"
      role="radio"
      aria-checked={a.value === value}
      aria-label={a.label}
      title={a.label}
      class="grid size-9 place-items-center rounded-md border transition-colors hover:bg-accent aria-checked:border-primary aria-checked:bg-primary/15"
      onclick={() => a.value !== value && onchange(a.value)}
    >
      <span
        class="size-2.5 rounded-sm {a.value === value ? 'bg-primary' : 'bg-muted-foreground/40'}"
      ></span>
    </button>
  {/each}
</div>
