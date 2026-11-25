<script lang="ts">
  // 「統計」のタブ。確定した字幕から数えた、喋った量と、よく呼んだヒーロー・アイテム・スキル。
  // 数だけをこの PC に保存する（文そのものは残さない）
  import * as Card from "$lib/components/ui/card";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import type { EntityKind } from "../../generated/EntityKind";
  import { entityName, glossary } from "../../lib/glossary.svelte";
  import { panel } from "../panel.svelte";

  type Period = "session" | "month";
  const PERIODS: { value: Period; label: string }[] = [
    { value: "session", label: "起動してから" },
    { value: "month", label: "今月" },
  ];
  const KINDS: { kind: EntityKind; title: string }[] = [
    { kind: "hero", title: "ヒーロー" },
    { kind: "item", title: "アイテム" },
    { kind: "ability", title: "スキル" },
  ];
  const TOP = 5;

  let period = $state<Period>("session");
  const stats = $derived(panel.stats?.[period]);
  const lang = $derived(panel.settings?.micLanguage ?? "ja");
  // 空白で区切らない言語は文字数、ほかは語の数で見せる
  const byCharacter = $derived(["ja", "zh"].some((l) => lang === l || lang.startsWith(`${l}-`)));
  const amount = $derived(
    byCharacter
      ? `${(stats?.characters ?? 0).toLocaleString()} 文字`
      : `${(stats?.words ?? 0).toLocaleString()} 語`,
  );

  /** 種類ごとの、呼んだ回数の多い順 */
  function ranking(kind: EntityKind) {
    return Object.entries(stats?.names ?? {})
      .filter(([ref]) => ref.startsWith(`${kind}:`))
      .toSorted(([, a], [, b]) => b - a)
      .slice(0, TOP)
      .map(([ref, count]) => ({ ref, count, entity: glossary.entity(ref) }));
  }
</script>

<div class="flex flex-col gap-6">
  <ToggleGroup.Root
    type="single"
    variant="outline"
    class="w-full"
    value={period}
    onValueChange={(v) => v && (period = v as Period)}
  >
    {#each PERIODS as p (p.value)}
      <ToggleGroup.Item value={p.value} class="flex-1">{p.label}</ToggleGroup.Item>
    {/each}
  </ToggleGroup.Root>

  <Card.Root>
    <Card.Header>
      <Card.Title>喋った量</Card.Title>
      <Card.Description>
        確定した字幕から数えます。数だけをこの PC に保存し、文そのものは残しません。今月は UTC
        の月です
      </Card.Description>
    </Card.Header>
    <Card.Content>
      <div class="grid grid-cols-2 gap-4">
        <div>
          <div class="text-xs text-muted-foreground">字幕の数</div>
          <div class="text-2xl font-medium tabular-nums">
            {(stats?.utterances ?? 0).toLocaleString()}
          </div>
        </div>
        <div>
          <div class="text-xs text-muted-foreground">喋った量</div>
          <div class="text-2xl font-medium tabular-nums">{amount}</div>
        </div>
      </div>
    </Card.Content>
  </Card.Root>

  <Card.Root>
    <Card.Header>
      <Card.Title>よく呼んだ名前</Card.Title>
      <Card.Description>略称で呼んでも、同じものとして数えます</Card.Description>
    </Card.Header>
    <Card.Content class="grid gap-6 sm:grid-cols-3">
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
    </Card.Content>
  </Card.Root>
</div>
