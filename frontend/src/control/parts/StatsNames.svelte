<script lang="ts">
  // 呼んだヒーロー・アイテム・スキルの、種類ごとの上位（統計のタブの今月と、回ごと）
  import type { EntityKind } from "../../generated/EntityKind";
  import { entityName, glossary } from "../../lib/glossary.svelte";

  let { names, lang }: { names: Record<string, number>; lang: string } = $props();

  const KINDS: { kind: EntityKind; title: string }[] = [
    { kind: "hero", title: "ヒーロー" },
    { kind: "item", title: "アイテム" },
    { kind: "ability", title: "スキル" },
  ];
  const TOP = 5;

  /** 種類ごとの、呼んだ回数の多い順 */
  function ranking(kind: EntityKind) {
    return Object.entries(names)
      .filter(([ref]) => ref.startsWith(`${kind}:`))
      .toSorted(([, a], [, b]) => b - a)
      .slice(0, TOP)
      .map(([ref, count]) => ({ ref, count, entity: glossary.entity(ref) }));
  }
</script>

<div class="grid gap-6 sm:grid-cols-3">
  {#each KINDS as k (k.kind)}
    {@const rows = ranking(k.kind)}
    <div>
      <div class="mb-2 text-sm font-medium">{k.title}</div>
      {#if rows.length === 0}
        <div class="text-sm text-muted-foreground">まだありません</div>
      {:else}
        <ol class="flex flex-col gap-1.5 text-sm">
          {#each rows as r (r.ref)}
            <li class="flex items-center gap-2">
              {#if r.entity?.icon}
                <img src={r.entity.icon} alt="" class="size-5 shrink-0 object-contain" />
              {/if}
              <span class="min-w-0 flex-1 truncate">
                {r.entity ? entityName(r.entity, lang) : r.ref}
              </span>
              <span class="tabular-nums text-muted-foreground">{r.count} 回</span>
            </li>
          {/each}
        </ol>
      {/if}
    </div>
  {/each}
</div>
