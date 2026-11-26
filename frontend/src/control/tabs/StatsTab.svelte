<script lang="ts">
  import ChevronDownIcon from "@lucide/svelte/icons/chevron-down";
  // 「統計」のタブ。字幕を始めてから止めるまでを 1 回として、確定した字幕から数えた、
  // 字幕を出していた時間、喋った量と、よく呼んだヒーロー・アイテム・スキル。
  // 上から、前回（出している間は今回）、今週・今月・ぜんぶの合計、日ごとの草、時間帯の表、回ごとの一覧。
  // 数だけをこの PC に保存する（文そのものは残さない）
  import ConfirmDialog from "../parts/ConfirmDialog.svelte";
  import Help from "../parts/Help.svelte";
  import ChevronRightIcon from "@lucide/svelte/icons/chevron-right";
  import TrashIcon from "@lucide/svelte/icons/trash-2";
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import type { Session } from "../../generated/Session";
  import type { Totals } from "../../generated/Totals";
  import StatsGrass from "../parts/StatsGrass.svelte";
  import StatsHours from "../parts/StatsHours.svelte";
  import StatsNames from "../parts/StatsNames.svelte";
  import { count, hoursAndMinutes } from "../parts/stats";
  import { panel } from "../panel.svelte";

  const stats = $derived(panel.stats);
  /** 新しい順に、この窓が頼んだ数まで（「もっと見る」で増える。数を決めて送るのは core） */
  const sessions = $derived(stats?.sessions ?? []);
  /** 消すかを確かめている回 */
  let forgetting = $state<Session>();
  let confirmingSession = $state(false);
  let confirmingAll = $state(false);

  const PERIODS = [
    { value: "week", label: "今週" },
    { value: "month", label: "今月" },
    { value: "all", label: "ぜんぶ" },
  ] as const;
  type Period = (typeof PERIODS)[number]["value"];
  let period = $state<Period>("week");
  const totals = $derived(stats?.[period]);
  const live = $derived(stats?.sessions.find((s) => s.live));
  /** 一番新しい回。出している間は、その回 */
  const last = $derived(stats?.sessions[0]);
  /** 今出している回は、最後に字幕が確定したところまでしか数えていないので、今までを足す */
  const duration = $derived(
    totals
      ? totals.durationMs +
          (live && live.startedAt >= totals.since ? Math.max(0, panel.now - live.endedAt) : 0)
      : 0,
  );
  /** 草は、今週の月曜から 52 週前の月曜から */
  const grassSince = $derived.by(() => {
    if (!stats) return 0;
    const monday = new Date(stats.week.since);
    return new Date(monday.getFullYear(), monday.getMonth(), monday.getDate() - 364).getTime();
  });
  const lang = $derived(panel.settings?.micLanguage ?? "ja");
  // 空白で区切らない言語は文字数、ほかは語の数で見せる
  const byCharacter = $derived(
    ["ja", "zh", "th", "lo", "my", "km"].some((l) => lang === l || lang.startsWith(`${l}-`)),
  );
  const amount = (t: Totals) =>
    byCharacter ? `${count(t.characters)} 文字` : `${count(t.words)} 語`;

  const when = new Intl.DateTimeFormat("ja-JP", {
    month: "numeric",
    day: "numeric",
    weekday: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
  const until = new Intl.DateTimeFormat("ja-JP", { hour: "2-digit", minute: "2-digit" });
  /** 始めた日時から、止めた時刻まで（日付が変わっていたら、止めた日も） */
  function span(s: Session) {
    if (s.live) return `${when.format(s.startedAt)}〜`;
    const sameDay = new Date(s.startedAt).toDateString() === new Date(s.endedAt).toDateString();
    return `${when.format(s.startedAt)}〜${(sameDay ? until : when).format(s.endedAt)}`;
  }
</script>

{#snippet tile(label: string, value: string)}
  <div class="min-w-0">
    <div class="text-xs text-muted-foreground">{label}</div>
    <div class="truncate text-2xl font-medium tabular-nums" title={value}>{value}</div>
  </div>
{/snippet}

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title class="flex items-center gap-1.5"
        >{last?.live ? "今回" : "前回"}<Help
          >字幕を始めてから止めるまでを 1 回にします。字幕を出している間は、今の回が出ます</Help
        ></Card.Title
      >
      {#if last}
        <Card.Description>
          {span(last)}
        </Card.Description>
      {/if}
    </Card.Header>
    <Card.Content class="flex flex-col gap-6">
      {#if !last}
        <div class="text-sm text-muted-foreground">まだありません</div>
      {:else}
        <div class="grid grid-cols-2 gap-4 sm:grid-cols-3">
          {@render tile(
            "字幕を出していた時間",
            hoursAndMinutes((last.live ? panel.now : last.endedAt) - last.startedAt),
          )}
          {@render tile("喋った量", amount(last))}
          {@render tile("字幕の数", count(last.utterances))}
        </div>
        <StatsNames names={last.names} {lang} />
      {/if}
    </Card.Content>
  </Card.Root>

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
      <div class="grid grid-cols-2 gap-4 sm:grid-cols-4">
        {@render tile("字幕を出していた時間", hoursAndMinutes(duration))}
        {@render tile("喋った量", totals ? amount(totals) : "—")}
        {@render tile("字幕の数", count(totals?.utterances ?? 0))}
        {@render tile("字幕を出した日", `${count(totals?.activeDays ?? 0)} 日`)}
      </div>
      <StatsNames names={totals?.names ?? {}} {lang} />
    </Card.Content>
  </Card.Root>

  <Card.Root>
    <Card.Header>
      <Card.Title class="flex items-center gap-1.5"
        >日ごと<Help
          >直近 1 年の、日ごとの喋った量です。多い日ほど濃くなります。回は始めた日に数えます</Help
        ></Card.Title
      >
    </Card.Header>
    <Card.Content>
      <StatsGrass days={stats?.days ?? []} since={grassSince} now={panel.now} {byCharacter} />
    </Card.Content>
  </Card.Root>

  <Card.Root>
    <Card.Header>
      <Card.Title class="flex items-center gap-1.5"
        >時間帯<Help>直近 90 日に、字幕を出していた時間です。長い曜日と時間ほど濃くなります</Help
        ></Card.Title
      >
    </Card.Header>
    <Card.Content>
      <StatsHours hours={stats?.hours ?? []} />
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
      {#if !sessions.length}
        <div class="text-sm text-muted-foreground">まだありません</div>
      {:else}
        <ul class="flex flex-col divide-y">
          {#each sessions as s (s.id)}
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
                        {span(s)}
                      </span>
                      {#if s.live}<Badge>字幕を出しています</Badge>{/if}
                    </div>
                    <div class="text-xs text-muted-foreground">
                      {hoursAndMinutes((s.live ? panel.now : s.endedAt) - s.startedAt)}
                    </div>
                  </div>
                  <div class="text-right tabular-nums">
                    <div>{amount(s)}</div>
                    <div class="text-xs text-muted-foreground">
                      字幕 {count(s.utterances)}
                    </div>
                  </div>
                </summary>
                <div class="flex flex-col items-start gap-3 pb-4 pl-7">
                  <StatsNames names={s.names} {lang} />
                  <Button
                    variant="ghost"
                    size="sm"
                    class="-ml-2.5 text-muted-foreground"
                    onclick={() => {
                      forgetting = s;
                      confirmingSession = true;
                    }}><TrashIcon />この回を削除</Button
                  >
                </div>
              </details>
            </li>
          {/each}
        </ul>
        {#if sessions.length >= panel.statsSessions}
          <Button
            variant="outline"
            size="sm"
            class="mt-2 w-full"
            onclick={() => panel.showMoreSessions()}><ChevronDownIcon />もっと見る</Button
          >
        {/if}
      {/if}
    </Card.Content>
  </Card.Root>

  {#if sessions.length}
    <div class="flex justify-end">
      <Button
        variant="ghost"
        size="sm"
        class="text-muted-foreground"
        onclick={() => (confirmingAll = true)}><TrashIcon />統計をすべて削除</Button
      >
    </div>
  {/if}
</div>

<ConfirmDialog
  bind:open={confirmingSession}
  title="この回を削除しますか？"
  action="削除"
  destructive
  onconfirm={() => (forgetting ? panel.forgetSession(forgetting.id) : Promise.resolve(true))}
>
  {#if forgetting}
    <p>{span(forgetting)} の回を消します。合計や日ごとの草からも外れます。元には戻せません。</p>
    {#if forgetting.live}<p>今出している字幕は、続きを新しい回として数えます。</p>{/if}
  {/if}
</ConfirmDialog>

<ConfirmDialog
  bind:open={confirmingAll}
  title="統計をすべて削除しますか？"
  action="すべて削除"
  destructive
  onconfirm={panel.forgetStats}
>
  <p>これまでのすべての回を消します。元には戻せません。</p>
</ConfirmDialog>
