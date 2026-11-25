<script lang="ts">
  // 統計の「日ごと」。52 週前の月曜から今日まで、1 日 1 マスで、喋った量の多い日ほど濃くする（GitHub の草のように）
  import type { StatsDay } from "../../generated/StatsDay";
  import { HEAT, heat } from "./stats";

  let {
    days,
    since,
    now,
    byCharacter,
  }: { days: StatsDay[]; since: number; now: number; byCharacter: boolean } = $props();

  const key = (d: Date) => `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
  const format = new Intl.DateTimeFormat("ja-JP", {
    year: "numeric",
    month: "numeric",
    day: "numeric",
    weekday: "short",
  });

  const amountOf = (d: StatsDay) => (byCharacter ? d.characters : d.words);
  const unit = $derived(byCharacter ? "文字" : "語");

  /** 週ごと（列）に、月曜から日曜までの 7 日。今日より後は null */
  const weeks = $derived.by(() => {
    const byDay = new Map(days.map((d) => [key(new Date(d.start)), d]));
    const today = new Date(now);
    const start = new Date(since);
    const columns: { date: Date; day?: StatsDay }[][] = [];
    for (let i = 0; ; i++) {
      // 日付で進める（夏時間で 1 日が 24 時間でなくても、ずれない）
      const date = new Date(start.getFullYear(), start.getMonth(), start.getDate() + i);
      if (date > today) break;
      if (i % 7 === 0) columns.push([]);
      columns.at(-1)!.push({ date, day: byDay.get(key(date)) });
    }
    return columns;
  });
  const max = $derived(Math.max(0, ...days.map(amountOf)));
  /** 1 日のある週の上に、月の名前を出す（最初の週から出すと、次の月の名前と重なる） */
  const monthAt = (week: { date: Date }[]) => {
    const first = week.find((c) => c.date.getDate() === 1);
    return first ? `${first.date.getMonth() + 1}月` : "";
  };
</script>

<div class="flex flex-col gap-2">
  <!-- 幅に合わせてマスを縮める（狭くても、新しい週まで見える） -->
  <div
    class="grid grid-flow-col grid-rows-[auto_repeat(7,auto)] gap-[3px] text-[10px]"
    style:grid-template-columns="auto repeat({weeks.length}, minmax(0, 1fr))"
  >
    <div></div>
    {#each ["月", "", "水", "", "金", "", ""] as label, i (i)}
      <div class="self-center pr-1 leading-none text-muted-foreground">{label}</div>
    {/each}
    {#each weeks as week, i (i)}
      <div class="leading-none whitespace-nowrap text-muted-foreground">{monthAt(week)}</div>
      {#each week as cell (cell.date.getTime())}
        {@const value = cell.day ? amountOf(cell.day) : 0}
        <div
          class="aspect-square rounded-[2px] {heat(value, max)}"
          title="{format.format(cell.date)} {value.toLocaleString()} {unit}"
        ></div>
      {/each}
    {/each}
  </div>
  <div class="flex items-center justify-end gap-1 text-[10px] text-muted-foreground">
    少ない
    {#each HEAT as level (level)}<div class="size-[10px] rounded-[2px] {level}"></div>{/each}
    多い
  </div>
</div>
