<script lang="ts">
  // モード（おすすめの組み合わせ）を選ぶ。自分で組んだ組み合わせは「カスタム」として 1 つ覚えている（DESIGN.md 9.6）
  import { Badge } from "$lib/components/ui/badge";
  import { panel } from "../panel.svelte";
  import { MODES, keyNames, modeOf, monthly, paidItems, price, providers } from "../../lib/engines";

  const s = $derived(panel.settings!);
  const t = $derived(s.engines);
  const mode = $derived(modeOf(t));
  const paid = $derived(paidItems(t));
  /** 覚えているカスタム。今がカスタムなら、今の組み合わせ */
  const custom = $derived(mode ? s.customEngines : t);
  const card =
    "flex flex-col gap-1 rounded-lg border p-3 text-left transition-colors hover:bg-muted aria-checked:border-foreground aria-checked:bg-muted";
</script>

<div class="flex flex-col gap-3">
  <div
    class="grid grid-cols-[repeat(auto-fit,minmax(9.5rem,1fr))] gap-2"
    role="radiogroup"
    aria-label="モード"
  >
    {#each MODES as m (m.name)}
      <button
        type="button"
        role="radio"
        aria-checked={mode?.name === m.name}
        class={card}
        onclick={() => panel.chooseMode(m.engines)}
      >
        <span class="flex items-center gap-2 text-sm font-medium">
          {m.label}
          {#if m.recommended}<Badge variant="secondary">おすすめ</Badge>{/if}
        </span>
        <span class="text-xs text-muted-foreground">{m.traits}</span>
        <span class="text-xs text-muted-foreground tabular-nums">{price(m.hourlyUsd)}</span>
        {#if m.hourlyUsd}
          <span class="text-xs text-muted-foreground tabular-nums">{monthly(m.hourlyUsd)}</span>
        {/if}
      </button>
    {/each}
    {#if custom}
      <button
        type="button"
        role="radio"
        aria-checked={!mode}
        class={card}
        onclick={() => panel.chooseMode(custom)}
      >
        <span class="text-sm font-medium">カスタム</span>
        <span class="text-xs text-muted-foreground">工程ごとに選んだ組み合わせ</span>
      </button>
    {/if}
  </div>
  <p class="text-sm text-muted-foreground">
    {#if mode}
      {mode.hint}
    {:else if paid.length}
      工程ごとに選んだ組み合わせです。鍵の要る工程: {paid.join("、")}
    {:else}
      工程ごとに選んだ組み合わせです。鍵は要りません
    {/if}{#if !mode && paid.length}。{keyNames(providers(t))}が要ります{/if}
  </p>
</div>
