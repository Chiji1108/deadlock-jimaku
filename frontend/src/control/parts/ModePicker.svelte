<script lang="ts">
  // モード（おすすめの組み合わせ）を選ぶ。下の工程と同じ、ラジオボタンの一覧。
  // 自分で組んだ組み合わせは「カスタム」として 1 つ覚えている（DESIGN.md 9.6）。
  // 選ぶ人が限られるモードは、「その他のモード」を開くと出る
  import ChevronIcon from "@lucide/svelte/icons/chevron-down";
  import { Badge } from "$lib/components/ui/badge";
  import { panel } from "../panel.svelte";
  import { MODES, keyNames, modeOf, monthly, paidItems, price, providers } from "../../lib/engines";

  const s = $derived(panel.settings!);
  const t = $derived(s.engines);
  const mode = $derived(modeOf(t));
  const paid = $derived(paidItems(t));
  /** 覚えているカスタム。今がカスタムなら、今の組み合わせ */
  const custom = $derived(mode ? s.customEngines : t);
  /** 「その他のモード」を開いている。その中のモードを使っている間は、開いたまま */
  let opened = $state(false);
  const more = $derived(opened || !!mode?.more);
  const row =
    "flex flex-col gap-1 p-3 text-left transition-colors first:rounded-t-lg last:rounded-b-lg hover:bg-muted aria-checked:bg-muted";
</script>

{#snippet dot(checked: boolean)}
  <span
    class="size-3 shrink-0 rounded-full border border-muted-foreground"
    class:bg-foreground={checked}
    class:border-foreground={checked}
  ></span>
{/snippet}

{#snippet option(m: (typeof MODES)[number])}
  <button
    type="button"
    role="radio"
    aria-checked={mode?.name === m.name}
    class={row}
    onclick={() => panel.chooseMode(m.engines)}
  >
    <span class="flex flex-wrap items-center gap-2 text-sm">
      {@render dot(mode?.name === m.name)}
      <span class="font-medium">{m.label}</span>
      <!-- 「ベータ期間中は無料」のようなひとことがあれば、料金の代わりに出す -->
      {#if m.badge}
        <Badge variant="secondary">{m.badge}</Badge>
      {:else}
        <span class="text-xs text-muted-foreground tabular-nums">
          {price(m.hourlyUsd)}{#if m.hourlyUsd}（{monthly(m.hourlyUsd)}）{/if}
        </span>
      {/if}
    </span>
    <span class="pl-5 text-xs text-muted-foreground">{m.traits}。{m.hint}</span>
  </button>
{/snippet}

<div class="flex flex-col divide-y rounded-lg border" role="radiogroup" aria-label="モード">
  {#each MODES.filter((m) => !m.more) as m (m.name)}
    {@render option(m)}
  {/each}
  {#if custom}
    <button
      type="button"
      role="radio"
      aria-checked={!mode}
      class={row}
      onclick={() => panel.chooseMode(custom)}
    >
      <span class="flex flex-wrap items-center gap-2 text-sm">
        {@render dot(!mode)}
        <span class="font-medium">カスタム</span>
      </span>
      <span class="pl-5 text-xs text-muted-foreground">
        下の工程ごとに選んだ組み合わせ{#if !mode}{#if paid.length}。鍵の要る工程: {paid.join(
              "、",
            )}（{keyNames(providers(t))}が要ります）{:else}。鍵は要りません{/if}{/if}
      </span>
    </button>
  {/if}
  <button
    type="button"
    class="flex items-center gap-2 p-3 text-left text-xs text-muted-foreground transition-colors last:rounded-b-lg hover:bg-muted"
    aria-expanded={more}
    disabled={!!mode?.more}
    onclick={() => (opened = !opened)}
  >
    <ChevronIcon class={["size-3.5 transition-transform", more && "rotate-180"]} />
    その他のモード（Chrome を使わない、PC の中だけで作る）
  </button>
  {#if more}
    {#each MODES.filter((m) => m.more) as m (m.name)}
      {@render option(m)}
    {/each}
  {/if}
</div>
