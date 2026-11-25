<script lang="ts">
  // 「統計」のタブ。字幕を始めてから止めるまでを 1 回として、確定した字幕から数えた、
  // 喋った量と、よく呼んだヒーロー・アイテム・スキル。上に今日・今週・今月・ぜんぶの合計、下に回ごとの一覧。
  // 数だけをこの PC に保存する（文そのものは残さない）
  import Help from "../parts/Help.svelte";
  import ChevronRightIcon from "@lucide/svelte/icons/chevron-right";
  import { Badge } from "$lib/components/ui/badge";
  import * as Card from "$lib/components/ui/card";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import type { Totals } from "../../generated/Totals";
  import StatsNames from "../parts/StatsNames.svelte";
  import { panel } from "../panel.svelte";

  const stats = $derived(panel.stats);

  const PERIODS = [
    { value: "today", label: "今日" },
    { value: "week", label: "今週" },
    { value: "month", label: "今月" },
    { value: "all", label: "ぜんぶ" },
  ] as const;
  type Period = (typeof PERIODS)[number]["value"];
  let period = $state<Period>("today");
  const totals = $derived(stats?.[period]);
  const lang = $derived(panel.settings?.micLanguage ?? "ja");
  // 空白で区切らない言語は文字数、ほかは語の数で見せる
  const byCharacter = $derived(
    ["ja", "zh", "th", "lo", "my", "km"].some((l) => lang === l || lang.startsWith(`${l}-`)),
  );
  const amount = (t: Totals) =>
    byCharacter ? `${t.characters.toLocaleString()} 文字` : `${t.words.toLocaleString()} 語`;

  const when = new Intl.DateTimeFormat("ja-JP", {
    month: "numeric",
    day: "numeric",
    weekday: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
  const until = new Intl.DateTimeFormat("ja-JP", { hour: "2-digit", minute: "2-digit" });
  function length(ms: number) {
    const minutes = Math.max(1, Math.round(ms / 60_000));
    return minutes < 60 ? `${minutes} 分` : `${Math.floor(minutes / 60)} 時間 ${minutes % 60} 分`;
  }
</script>

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title class="flex items-center gap-1.5"
        >合計<Help
          >その期間に始めた回の合計です。週は月曜から数えます。確定した字幕から数えます。数だけをこの
          PC
          に保存し、文そのものは残しません。名前は、字幕で用語を強調しているときだけ数えます（略称で呼んでも同じものとして数えます）</Help
        ></Card.Title
      >
    </Card.Header>
    <Card.Content class="flex flex-col gap-6">
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
      <div class="grid grid-cols-2 gap-4">
        <div>
          <div class="text-xs text-muted-foreground">字幕の数</div>
          <div class="text-2xl font-medium tabular-nums">
            {(totals?.utterances ?? 0).toLocaleString()}
          </div>
        </div>
        <div>
          <div class="text-xs text-muted-foreground">喋った量</div>
          <div class="text-2xl font-medium tabular-nums">
            {totals ? amount(totals) : "—"}
          </div>
        </div>
      </div>
      <StatsNames names={totals?.names ?? {}} {lang} />
    </Card.Content>
  </Card.Root>

  <Card.Root>
    <Card.Header>
      <Card.Title class="flex items-center gap-1.5"
        >回ごと<Help
          >字幕を始めてから止めるまでを 1
          回にします。何も字幕が出なかった回は残しません。押すと、その回に呼んだ名前が出ます</Help
        ></Card.Title
      >
    </Card.Header>
    <Card.Content>
      {#if !stats?.sessions.length}
        <div class="text-sm text-muted-foreground">まだありません</div>
      {:else}
        <ul class="flex flex-col divide-y">
          {#each stats.sessions as s (s.id)}
            <li>
              <details class="group">
                <summary
                  class="flex cursor-pointer list-none items-center gap-3 py-3 text-sm [&::-webkit-details-marker]:hidden"
                >
                  <ChevronRightIcon
                    class="size-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-90"
                  />
                  <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2">
                      <span class="font-medium">
                        {when.format(s.startedAt)}〜{s.live ? "" : until.format(s.endedAt)}
                      </span>
                      {#if s.live}<Badge>字幕を出しています</Badge>{/if}
                    </div>
                    <div class="text-xs text-muted-foreground">
                      {length((s.live ? panel.now : s.endedAt) - s.startedAt)}
                    </div>
                  </div>
                  <div class="text-right tabular-nums">
                    <div>{amount(s)}</div>
                    <div class="text-xs text-muted-foreground">
                      字幕 {s.utterances.toLocaleString()}
                    </div>
                  </div>
                </summary>
                <div class="pb-4 pl-7">
                  <StatsNames names={s.names} {lang} />
                </div>
              </details>
            </li>
          {/each}
        </ul>
      {/if}
    </Card.Content>
  </Card.Root>
</div>
