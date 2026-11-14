<script lang="ts">
  // モード（おすすめの組み合わせ）を選ぶ。下の工程と同じ、ラジオボタンの一覧。
  // 選ぶ人が限られるモードは、「その他のモード」を開くと出る。
  // 自分で組んだ組み合わせ（カスタム）は、選ぶものではなく今の状態なので、一覧には入れず、下に 1 行で出す。
  // 1 つ覚えているので、モードを試したあとに戻れる（DESIGN.md 9.6）
  import ChevronIcon from "@lucide/svelte/icons/chevron-down";
  import { Button } from "$lib/components/ui/button";
  import { panel } from "../panel.svelte";
  import ModeBadges from "./ModeBadges.svelte";
  import { MODES, keyNames, modeOf, modeStats, paidItems, providers } from "../../lib/engines";

  const s = $derived(panel.settings!);
  const t = $derived(s.engines);
  const mode = $derived(modeOf(t));
  const paid = $derived(paidItems(t));
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
      <ModeBadges mode={m} />
    </span>
    <!-- どのモードも同じ並びで、工程ごとの正確さを出す（見比べられるように） -->
    <span class="flex flex-wrap gap-x-4 pl-5 text-xs tabular-nums">
      {#each modeStats(m.engines) as stat (stat.label)}
        <span><span class="text-muted-foreground">{stat.label}</span> {stat.value}</span>
      {/each}
    </span>
    <span class="pl-5 text-xs text-muted-foreground">{m.hint}</span>
  </button>
{/snippet}

<div class="flex flex-col divide-y rounded-lg border" role="radiogroup" aria-label="モード">
  {#each MODES.filter((m) => !m.more) as m (m.name)}
    {@render option(m)}
  {/each}
  <button
    type="button"
    class="flex items-center gap-2 p-3 text-left text-xs text-muted-foreground transition-colors last:rounded-b-lg hover:bg-muted"
    aria-expanded={more}
    disabled={!!mode?.more}
    onclick={() => (opened = !opened)}
  >
    <ChevronIcon class={["size-3.5 transition-transform", more && "rotate-180"]} />
    その他のモード（最高精度、クラウド、ローカル）
  </button>
  {#if more}
    {#each MODES.filter((m) => m.more) as m (m.name)}
      {@render option(m)}
    {/each}
  {/if}
</div>
{#if !mode}
  <p class="mt-3 rounded-lg bg-muted p-3 text-xs text-muted-foreground">
    <span class="font-medium text-foreground">カスタム</span>
    工程ごとに選んだ組み合わせを使っています{#if paid.length}。鍵の要る工程: {paid.join(
        "、",
      )}（{keyNames(providers(t))}が要ります）{:else}。鍵は要りません{/if}
  </p>
{:else if s.customEngines}
  <div class="mt-2">
    <Button
      variant="link"
      size="sm"
      class="h-auto p-0 text-xs text-muted-foreground"
      onclick={() => panel.chooseMode(s.customEngines!)}
    >
      前に組んだカスタムに戻す
    </Button>
  </div>
{/if}
