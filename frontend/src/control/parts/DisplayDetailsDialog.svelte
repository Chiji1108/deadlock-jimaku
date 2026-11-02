<script lang="ts">
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import type { SubtitleDisplay } from "../../generated/SubtitleDisplay";
  import type { SubtitleDisplayPatch } from "../../generated/SubtitleDisplayPatch";
  import { panel } from "../panel.svelte";
  import ColorInput, { HEX } from "./ColorInput.svelte";
  import SettingsDialog from "./SettingsDialog.svelte";

  // 文字の大きさと色を数値で決める（範囲はサーバーと同じ）
  const SIZE = { min: 16, max: 96 };
  type Values = Pick<
    SubtitleDisplay,
    "fontSize" | "translationFontSize" | "textColor" | "heroColor" | "abilityColor"
  >;
  const pick = ({ fontSize, translationFontSize, textColor, heroColor, abilityColor }: Values) => ({
    fontSize,
    translationFontSize,
    textColor,
    heroColor,
    abilityColor,
  });

  const d = $derived(panel.settings!.subtitleDisplay);
  let values = $state<Values>(pick(panel.settings!.subtitleDisplay));
  /** 開いたときの値。変えた項目だけを送る（開いている間にほかで変えた項目を戻さない） */
  let opened = pick(panel.settings!.subtitleDisplay);

  const sizes = [
    { key: "fontSize", label: "話した言葉の大きさ" },
    { key: "translationFontSize", label: "翻訳の大きさ" },
  ] as const;
  const colors = [
    { key: "textColor", label: "文字の色" },
    { key: "heroColor", label: "ヒーロー名の色" },
    { key: "abilityColor", label: "スキル名の色" },
  ] as const;

  const valid = $derived(
    sizes.every(({ key }) => {
      const n = values[key];
      return Number.isInteger(n) && n >= SIZE.min && n <= SIZE.max;
    }) && colors.every(({ key }) => HEX.test(values[key])),
  );

  function reset() {
    opened = pick(d);
    values = pick(d);
  }

  function save() {
    const patch: SubtitleDisplayPatch = Object.fromEntries(
      Object.entries(values).filter(([k, v]) => opened[k as keyof Values] !== v),
    );
    return panel.update({ subtitleDisplay: patch });
  }
</script>

<SettingsDialog
  trigger="詳しく設定"
  title="文字の大きさと色"
  description="大きさは OBS のブラウザソースの中での px です（{SIZE.min}〜{SIZE.max}）。アイテム名はゲーム内のスロットの色のままです"
  {valid}
  onopen={reset}
  onsave={save}
>
  <Field.Group>
    {#each sizes as s (s.key)}
      <Field.Field orientation="horizontal">
        <Field.Label for={s.key} class="flex-1">{s.label}</Field.Label>
        <Input
          id={s.key}
          type="number"
          class="w-24"
          min={SIZE.min}
          max={SIZE.max}
          bind:value={values[s.key]}
        />
      </Field.Field>
    {/each}
    <Field.Separator />
    {#each colors as c (c.key)}
      <Field.Field orientation="horizontal">
        <Field.Label for={c.key} class="flex-1">{c.label}</Field.Label>
        <ColorInput id={c.key} bind:value={values[c.key]} fallback={d[c.key]} />
      </Field.Field>
    {/each}
  </Field.Group>
</SettingsDialog>
