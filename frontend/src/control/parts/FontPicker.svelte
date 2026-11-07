<script lang="ts">
  // フォントの 1 行と、選ぶダイアログ。ダイアログには見本を大きく描いたカードを並べる
  import { onMount } from "svelte";
  import CheckIcon from "@lucide/svelte/icons/check";
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Input } from "$lib/components/ui/input";
  import * as Select from "$lib/components/ui/select";
  import type { FontEntry } from "../../generated/FontEntry";
  import {
    cardTarget,
    fontSampleName,
    fontSections,
    formatSize,
    STANDARD,
  } from "../../lib/font-samples";
  import { cssString, fontSample } from "../../lib/overlay-fonts";
  import { syntheticBold, weightChoices } from "../../lib/text-style";
  import { panel, type FontTarget } from "../panel.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";

  let {
    target,
    language,
    label,
    standardWeight,
  }: {
    target: FontTarget;
    /** 見本の言語。Google Fonts の見出しはこの言語で絞る */
    language: string;
    /** 字幕は言語名、ランクは「ランク」 */
    label: string;
    /** 太さの標準 */
    standardWeight: number;
  } = $props();

  const families = $derived(panel.fonts);
  const value = $derived(panel.chosenFont(target) ?? "");
  const weight = $derived(panel.fontStyle(target)?.weight ?? undefined);
  const faces = $derived(panel.weights.get(value));
  const weights = $derived(weightChoices(faces, standardWeight, weight));
  const weightLabel = (w: number) => `${w}${syntheticBold(faces, w) ? "（合成）" : ""}`;
  const standardLabel = $derived(
    `標準（${standardWeight}${syntheticBold(faces, standardWeight) ? "・合成" : ""}）`,
  );

  $effect(() => {
    if (value) void panel.loadWeights(value);
  });

  const HEADING = "pt-2 text-xs font-medium text-muted-foreground";
  const GRID = "grid grid-cols-1 gap-2 sm:grid-cols-2";
  /** 一度に描く数。PC のフォントが多いと描くだけで待たせるので、ほかは検索で絞ってもらう */
  const MAX_SHOWN = 500;
  const swatch = $derived(fontSample(language));
  let open = $state(false);
  let query = $state("");
  let search = $state<HTMLInputElement | null>(null);
  let list = $state<HTMLElement>();
  const words = $derived(query.trim().toLowerCase());
  const matches = $derived(
    typeof families === "string"
      ? []
      : families.filter((f) => f.family.toLowerCase().includes(words)),
  );
  const { googleFonts, others } = $derived(fontSections(matches, language));
  const showStandard = $derived(STANDARD.includes(words));
  const note = $derived.by(() => {
    if (families === "loading") return "読み込み中…";
    if (families === "failed") return "読み込めませんでした。開き直すと読み直します";
    if (!showStandard && googleFonts.length + others.length === 0) return "見つかりません";
    return others.length > MAX_SHOWN ? "検索するとほかのフォントも出ます" : undefined;
  });

  /** 確認しているフォント */
  let pending = $state<FontEntry>();

  function choose(font?: FontEntry) {
    if (font?.sizeBytes !== undefined) {
      pending = font;
      return;
    }
    open = false;
    void panel.setFont(target, font?.family ?? "");
  }

  /** 裏で進めるので、ダイアログはすぐ閉じる。適用と失敗の案内は panel が持つ */
  function download(family: string) {
    open = false;
    void panel.downloadFont(family, target);
    return Promise.resolve(true);
  }

  onMount(() => void panel.loadFonts());

  function onkeydown(e: KeyboardEvent) {
    const cards = () => [...list!.querySelectorAll<HTMLButtonElement>("button:enabled")];
    if (e.key === "Enter" && e.target === search && words && !e.isComposing) cards()[0]?.click();
    if (!e.key.startsWith("Arrow")) return;
    const shown = cards();
    const at = shown.indexOf(document.activeElement as HTMLButtonElement);
    const target = cardTarget(
      shown.map((card) => card.offsetLeft),
      at,
      e.key,
    );
    if (target === undefined) return;
    e.preventDefault();
    (shown[target] ?? search)?.focus();
  }
</script>

<!-- 標準は font が無い。--font を戻さないと、選んだフォントの規則（[lang][lang=…]）で描かれる -->
{#snippet card(font?: FontEntry)}
  {@const family = font?.family ?? ""}
  {@const fetching = panel.downloadingFonts.has(family)}
  <button
    type="button"
    aria-current={family === value}
    disabled={fetching}
    class="relative flex h-22 min-w-0 flex-col justify-between rounded-lg border p-3 text-left outline-none select-none [contain-intrinsic-size:auto_5.5rem] [content-visibility:auto] hover:bg-accent hover:text-accent-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:opacity-60 aria-[current=true]:border-primary"
    onclick={() => choose(font)}
  >
    {#if !font}
      <span
        aria-hidden="true"
        lang={language}
        class="overflow-clip text-[28px]/[1.3] whitespace-nowrap"
        style:--font="initial">{swatch}</span
      >
    {:else if font.sizeBytes === undefined}
      <!-- 標準の候補に続けない。無い字が別のフォントで描かれると、このフォントの見本に見える -->
      <span
        aria-hidden="true"
        lang={language}
        class="overflow-clip text-[28px]/[1.3] whitespace-nowrap"
        style:font-family={cssString(font.family)}>{swatch}</span
      >
    {:else}
      <span aria-hidden="true" class="flex overflow-clip text-[28px]">
        {@html panel.fontSamples[`./font-samples/${fontSampleName(font.family)}`] ?? ""}
      </span>
    {/if}
    <span class="flex min-w-0 items-baseline justify-between gap-2 text-sm">
      <span class="truncate">{family || STANDARD}</span>
      {#if font?.sizeBytes !== undefined}
        <span class="shrink-0 text-xs text-muted-foreground">
          {fetching ? "ダウンロード中…" : `ダウンロード（${formatSize(font.sizeBytes)}）`}
        </span>
      {/if}
    </span>
    {#if family === value}<CheckIcon class="absolute top-3 right-3 size-4" />{/if}
  </button>
{/snippet}

<div class="flex items-center gap-4">
  <div class="min-w-0 flex-1">
    <p class="flex gap-2 text-sm">
      <span class="shrink-0 font-medium">{label}</span>
      <span class="truncate text-muted-foreground">{value || STANDARD}</span>
    </p>
    <!-- この行のフォントは --font で渡す。選んだフォントがなければ lang.css の標準の候補に落ちる -->
    <p
      lang={language}
      class="truncate text-lg font-bold"
      style:--font={value ? `${cssString(value)},` : "initial"}
      style:font-weight={weight ?? standardWeight}
    >
      {swatch}
    </p>
  </div>
  <Select.Root
    type="single"
    value={weight ? String(weight) : ""}
    onValueChange={(v) => void panel.setFontStyle(target, { weight: v ? Number(v) : null })}
    onOpenChange={(opening) => {
      if (opening && value) void panel.loadWeights(value);
    }}
  >
    <Select.Trigger class="w-36 shrink-0" aria-label="{label}の太さ">
      {weight ? weightLabel(weight) : standardLabel}
    </Select.Trigger>
    <Select.Content>
      <Select.Item value="">{standardLabel}</Select.Item>
      {#each weights as w (w)}
        <Select.Item value={String(w)}>{weightLabel(w)}</Select.Item>
      {/each}
    </Select.Content>
  </Select.Root>
  <Dialog.Root
    bind:open
    onOpenChange={(opening) => {
      if (!opening) return;
      query = "";
      void panel.loadFonts();
      void panel.loadFontSamples();
    }}
  >
    <Dialog.Trigger>
      {#snippet child({ props })}
        <Button variant="outline" size="sm" aria-label="{label}のフォントを変更" {...props}
          >変更</Button
        >
      {/snippet}
    </Dialog.Trigger>
    <Dialog.Content class="flex max-h-[calc(100%-2rem)] flex-col sm:max-w-2xl" {onkeydown}>
      <Dialog.Header>
        <Dialog.Title>{label}のフォント</Dialog.Title>
      </Dialog.Header>
      <Input bind:ref={search} bind:value={query} placeholder="フォントを検索" />
      <!-- 画面外のカードは content-visibility で描かず、見本のフォントも読み込ませない。カードの高さは固定なので、スクロールの量は変わらない -->
      <div bind:this={list} class="-mx-2 flex min-h-0 flex-col gap-2 overflow-y-auto px-2 pb-1">
        {#if showStandard}
          <div class={GRID}>{@render card()}</div>
        {/if}
        {#if googleFonts.length > 0}
          <p class={HEADING}>Google Fonts</p>
          <div class={GRID}>
            {#each googleFonts as font (font.family)}{@render card(font)}{/each}
          </div>
        {/if}
        {#if others.length > 0}
          <p class={HEADING}>この PC のフォント</p>
          <div class={GRID}>
            {#each others.slice(0, MAX_SHOWN) as font (font.family)}{@render card(font)}{/each}
          </div>
        {/if}
        {#if note}<p class="py-1.5 text-sm text-muted-foreground">{note}</p>{/if}
      </div>
    </Dialog.Content>
  </Dialog.Root>
</div>

{#if pending}
  <ConfirmDialog
    bind:open={() => pending !== undefined, (next) => !next && (pending = undefined)}
    title="{pending.family} をダウンロードしますか"
    action="ダウンロード"
    onconfirm={() => download(pending!.family)}
  >
    <p>大きさ: {formatSize(pending.sizeBytes!)}</p>
    <p>Google Fonts のフォントを GitHub から取得します。</p>
    <p>ダウンロードしたフォントはこの PC に保存し、OBS の字幕とランクにも使います。</p>
  </ConfirmDialog>
{/if}
