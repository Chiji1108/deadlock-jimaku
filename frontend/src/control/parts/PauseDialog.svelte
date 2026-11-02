<script lang="ts">
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import { panel } from "../panel.svelte";
  import SettingsDialog from "./SettingsDialog.svelte";

  // 文の区切りをミリ秒で決める（範囲はサーバーと同じ）
  const RANGE = { min: 200, max: 2000 };

  let ms = $state(0);
  /** 開いたときの値。変えていなければ送らない（開いている間にほかで変えた値を戻さない） */
  let opened = 0;
  const valid = $derived(Number.isInteger(ms) && ms >= RANGE.min && ms <= RANGE.max);
</script>

<SettingsDialog
  trigger="数値で指定"
  title="文の区切り"
  description="話し終えてから、何ミリ秒黙ったら 1 つの字幕として確定するか（{RANGE.min}〜{RANGE.max}）"
  {valid}
  onopen={() => (ms = opened = panel.settings!.segmentation.minSilenceMs)}
  onsave={async () => ms === opened || panel.update({ minSilenceMs: ms })}
>
  <Field.Field orientation="horizontal">
    <Field.Label for="pause" class="flex-1">黙っている長さ（ミリ秒）</Field.Label>
    <Input id="pause" type="number" class="w-28" min={RANGE.min} max={RANGE.max} bind:value={ms} />
  </Field.Field>
</SettingsDialog>
