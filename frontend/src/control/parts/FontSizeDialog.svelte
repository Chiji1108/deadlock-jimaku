<script lang="ts">
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import { panel } from "../panel.svelte";
  import SettingsDialog from "./SettingsDialog.svelte";

  // 文字の大きさを数値で決める（範囲はサーバーと同じ）
  const RANGE = { min: 16, max: 96 };

  let fontSize = $state(0);
  let translationFontSize = $state(0);
  const fits = (n: number) => Number.isInteger(n) && n >= RANGE.min && n <= RANGE.max;
</script>

<SettingsDialog
  trigger="数値で指定"
  title="文字の大きさ"
  description="OBS のブラウザソースの中での px です（{RANGE.min}〜{RANGE.max}）"
  valid={fits(fontSize) && fits(translationFontSize)}
  onopen={() => {
    ({ fontSize, translationFontSize } = panel.settings!.subtitleDisplay);
  }}
  onsave={() => panel.update({ subtitleDisplay: { fontSize, translationFontSize } })}
>
  <Field.Group>
    <Field.Field orientation="horizontal">
      <Field.Label for="font-size" class="flex-1">話した言葉（px）</Field.Label>
      <Input id="font-size" type="number" class="w-24" {...RANGE} bind:value={fontSize} />
    </Field.Field>
    <Field.Field orientation="horizontal">
      <Field.Label for="translation-font-size" class="flex-1">翻訳（px）</Field.Label>
      <Input
        id="translation-font-size"
        type="number"
        class="w-24"
        {...RANGE}
        bind:value={translationFontSize}
      />
    </Field.Field>
  </Field.Group>
</SettingsDialog>
