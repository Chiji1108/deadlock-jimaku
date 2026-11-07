<script lang="ts">
  // 検索できるフォントの一覧。先頭の「標準」は lang.css のまま
  import { Command, Popover } from "bits-ui";
  import CheckIcon from "@lucide/svelte/icons/check";
  import ChevronDownIcon from "@lucide/svelte/icons/chevron-down";
  import type { FontList } from "../panel.svelte";

  let {
    id,
    value = "",
    families,
    onchange,
  }: {
    id: string;
    value?: string;
    families: FontList;
    onchange: (family: string) => void;
  } = $props();

  const STANDARD = "標準";
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
    if (families === "failed") return "読み込めませんでした。アプリを開き直してください";
    if (families.length === 0) return "この PC のフォントが見つかりません";
    if (matches.length === 0) return "見つかりません";
    return matches.length > MAX_SHOWN ? "検索するとほかのフォントも出ます" : undefined;
  });

  function choose(family: string) {
    open = false;
    onchange(family);
  }
</script>

{#snippet option(family: string, label: string)}
  <Command.Item
    value={label}
    class="relative flex cursor-default items-center rounded-sm py-1.5 pr-8 pl-2 text-sm outline-hidden select-none data-selected:bg-accent data-selected:text-accent-foreground"
    onSelect={() => choose(family)}
  >
    {label}
    {#if family === value}<CheckIcon class="absolute right-2 size-4" />{/if}
  </Command.Item>
{/snippet}

<Popover.Root bind:open onOpenChange={() => (query = "")}>
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
