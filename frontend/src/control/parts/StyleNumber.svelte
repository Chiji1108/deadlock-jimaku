<script lang="ts">
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import SettingsDialog from "./SettingsDialog.svelte";

  // 数の 1 行。選択肢を並べ、足りない値は「数値で指定」で決める。標準を選ぶと標準に戻す
  let {
    label,
    value,
    standard,
    presets,
    percent = false,
    range,
    onchange,
  }: {
    label: string;
    value: number | null;
    standard: number;
    presets: number[];
    /** 値を % で見せる（em の比と濃さ） */
    percent?: boolean;
    /** 見せる単位での範囲（範囲はサーバーと同じ） */
    range: { min: number; max: number; step: number };
    /** 保存できたら true */
    onchange: (value: number | null) => Promise<boolean>;
  } = $props();

  const id = $props.id();
  const scale = $derived(percent ? 100 : 1);
  const unit = $derived(percent ? "%" : "");
  const shown = (v: number) => Math.round(v * scale * 100) / 100;
  const current = $derived(value ?? standard);
  const choices = $derived([...new Set([...presets, standard])].toSorted((a, b) => a - b));

  let number = $state(0);
  const valid = $derived(Number.isFinite(number) && number >= range.min && number <= range.max);

  const choose = (v: number) => onchange(v === standard ? null : v);
</script>

<Field.Field>
  <div class="flex items-center justify-between">
    <Field.Label>{label}</Field.Label>
    <SettingsDialog
      trigger="数値で指定"
      title={label}
      description="{range.min}〜{range.max}{unit} で決めます"
      {valid}
      onopen={() => (number = shown(current))}
      onsave={() => choose(number / scale)}
    >
      <Field.Field orientation="horizontal">
        <Field.Label for={id} class="flex-1">{label}{percent ? "（%）" : ""}</Field.Label>
        <Input {id} type="number" class="w-24" {...range} bind:value={number} />
      </Field.Field>
    </SettingsDialog>
  </div>
  <ToggleGroup.Root
    type="single"
    variant="outline"
    class="w-full"
    aria-label={label}
    value={String(current)}
    onValueChange={(v) => v && choose(Number(v))}
  >
    {#each choices as v (v)}
      <ToggleGroup.Item value={String(v)} class="flex-1">{shown(v)}{unit}</ToggleGroup.Item>
    {/each}
  </ToggleGroup.Root>
  {#if !choices.includes(current)}
    <Field.Description>数値で指定しています（{shown(current)}{unit}）</Field.Description>
  {/if}
</Field.Field>
