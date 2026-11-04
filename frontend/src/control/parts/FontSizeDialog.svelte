<script lang="ts">
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import { panel } from "../panel.svelte";
  import SettingsDialog from "./SettingsDialog.svelte";

  // 文字の大きさを数値で決める（範囲はサーバーと同じ）
  const RANGE = { min: 16, max: 96 };

  let { field, label }: { field: "fontSize" | "translationFontSize"; label: string } = $props();

  let size = $state(0);
  /** 開いたときの値。変えていなければ送らない（ほかの画面で変えた値を戻さない） */
  let opened = 0;
  const valid = $derived(Number.isInteger(size) && size >= RANGE.min && size <= RANGE.max);
</script>

<SettingsDialog
  trigger="数値で指定"
  title={label}
  description="OBS のブラウザソースの中での px です（{RANGE.min}〜{RANGE.max}）"
  {valid}
  onopen={() => (size = opened = panel.settings!.subtitleDisplay[field])}
  onsave={async () => size === opened || panel.update({ subtitleDisplay: { [field]: size } })}
>
  <Field.Field orientation="horizontal">
    <Field.Label for={field} class="flex-1">大きさ（px）</Field.Label>
    <Input id={field} type="number" class="w-24" {...RANGE} bind:value={size} />
  </Field.Field>
</SettingsDialog>
