<script lang="ts">
  // 検索できるフォントの一覧。先頭の「標準」は lang.css のまま
  import { Command, Popover } from "bits-ui";
  import CheckIcon from "@lucide/svelte/icons/check";
  import ChevronDownIcon from "@lucide/svelte/icons/chevron-down";
  import { cssString, fontSample } from "../../lib/overlay-fonts";
  import type { FontList } from "../panel.svelte";

  let {
    id,
    language,
    value = "",
    families,
    onopen,
    onchange,
  }: {
    id: string;
    language: string;
    value?: string;
    families: FontList;
    onopen: () => void;
    onchange: (family: string) => void;
  } = $props();

  const STANDARD = "標準";
  const swatch = $derived(fontSample(language));
  /** 一度に描く数。Command は項目が増えると 1 文字ごとに固まるので、多い PC では検索で絞ってもらう */
  const MAX_SHOWN = 500;
  let open = $state(false);
  let query = $state("");
  const words = $derived(query.trim().toLowerCase());
  const matches = $derived(
    typeof families === "string" ? [] : families.filter((f) => f.toLowerCase().includes(words)),
  );
  const note = $derived.by(() => {
    if (families === "loading") return "読み込み中…";
    if (families === "failed") return "読み込めませんでした。開き直すと読み直します";
    if (families.length === 0) return "この PC のフォントが見つかりません";
    if (matches.length === 0) return "見つかりません";
    return matches.length > MAX_SHOWN ? "検索するとほかのフォントも出ます" : undefined;
  });

  function choose(family: string) {
    open = false;
    query = "";
    onchange(family);
  }
</script>

<!-- 画面外の行は content-visibility で描かず、見本のフォントも読み込ませない。行の高さは固定なので、スクロールの量は変わらない -->
{#snippet option(family: string, label: string)}
  <Command.Item
    value={label}
    class="relative flex h-8 cursor-default items-baseline gap-3 rounded-sm pr-8 pl-2 text-sm leading-8 outline-hidden select-none [contain-intrinsic-size:auto_2rem] [content-visibility:auto] data-selected:bg-accent data-selected:text-accent-foreground"
    onSelect={() => choose(family)}
  >
    <span class="min-w-0 flex-1 truncate">{label}</span>
    {#if family}
      <!-- 標準の候補に続けない。無い字が別のフォントで描かれると、このフォントの見本に見える -->
      <span
        aria-hidden="true"
        lang={language}
        class="max-w-1/2 shrink-0 overflow-clip text-base/8 whitespace-nowrap"
        style:font-family={cssString(family)}>{swatch}</span
      >
    {/if}
    {#if family === value}<CheckIcon class="absolute top-2 right-2 size-4" />{/if}
  </Command.Item>
{/snippet}

<Popover.Root
  bind:open
  onOpenChange={(opening) => {
    query = "";
    if (opening) onopen();
  }}
>
  <Popover.Trigger
    {id}
    class="flex h-9 w-full items-center justify-between gap-1.5 rounded-md border border-input bg-transparent py-2 pr-2 pl-2.5 text-sm whitespace-nowrap shadow-xs outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 dark:bg-input/30 dark:hover:bg-input/50"
  >
    <span class="truncate">{value || STANDARD}</span>
    <ChevronDownIcon class="size-4 shrink-0 text-muted-foreground" />
  </Popover.Trigger>
  <Popover.Portal>
    <Popover.Content
      sideOffset={4}
      class="z-50 w-(--bits-popover-anchor-width) rounded-md bg-popover text-popover-foreground shadow-md ring-1 ring-foreground/10"
    >
      <Command.Root loop shouldFilter={false}>
        <Command.Input
          bind:value={query}
          class="w-full border-b bg-transparent px-2.5 py-2 text-sm outline-none placeholder:text-muted-foreground"
          placeholder="フォントを検索"
        />
        <Command.List class="max-h-64 overflow-y-auto p-1">
          <Command.Viewport>
            {#if !words}{@render option("", STANDARD)}{/if}
            {#each matches.slice(0, MAX_SHOWN) as family (family)}
              {@render option(family, family)}
            {/each}
            {#if note}<p class="px-2 py-1.5 text-sm text-muted-foreground">{note}</p>{/if}
          </Command.Viewport>
        </Command.List>
      </Command.Root>
    </Popover.Content>
  </Popover.Portal>
</Popover.Root>
