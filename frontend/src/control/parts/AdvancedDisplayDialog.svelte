<script lang="ts">
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import { panel } from "../panel.svelte";
  import SettingsDialog from "./SettingsDialog.svelte";

  // 字幕の入れ替わり方を決める秒数（範囲はサーバーと同じ）
  const RANGE = { min: 0, max: 30, step: 0.5 };
  const FIELDS = [
    {
      key: "minReadSeconds",
      label: "翻訳を読む時間（秒）",
      hint: "翻訳が出てから、次の字幕に押し出すまでの最短の時間です。既定は 3 秒",
    },
    {
      key: "maxWaitSeconds",
      label: "次の字幕を待たせる上限（秒）",
      hint: "前の字幕が読み終わらなくても、この時間で次の字幕に切り替えます。0 ならすぐ切り替えます。既定は 6 秒",
    },
  ] as const;

  let values = $state({ minReadSeconds: 0, maxWaitSeconds: 0 });
  /** 開いたときの値。変えた項目だけを送る（ほかの画面で変えた値を戻さない） */
  let opened = { minReadSeconds: 0, maxWaitSeconds: 0 };

  const fits = (n: number) => Number.isFinite(n) && n >= RANGE.min && n <= RANGE.max;
  const valid = $derived(FIELDS.every((f) => fits(values[f.key])));

  function open() {
    const { minReadSeconds, maxWaitSeconds } = panel.settings!.subtitleDisplay;
    opened = { minReadSeconds, maxWaitSeconds };
    values = { ...opened };
  }

  async function save() {
    const changed = FIELDS.filter((f) => values[f.key] !== opened[f.key]);
    if (changed.length === 0) return true;
    return panel.update({
      subtitleDisplay: Object.fromEntries(changed.map((f) => [f.key, values[f.key]])),
    });
  }
</script>

<SettingsDialog
  trigger="高度な設定"
  title="字幕の入れ替わり方"
  description="同時に出す数を超えたら、読み終えた字幕から消します。読み終えた字幕がなければ、次の字幕を待たせます"
  {valid}
  onopen={open}
  onsave={save}
>
  <Field.Group>
    {#each FIELDS as f (f.key)}
      <Field.Field>
        <div class="flex items-center gap-4">
          <Field.Label for={f.key} class="flex-1">{f.label}</Field.Label>
          <Input id={f.key} type="number" class="w-24" {...RANGE} bind:value={values[f.key]} />
        </div>
        <Field.Description>{f.hint}</Field.Description>
      </Field.Field>
    {/each}
  </Field.Group>
</SettingsDialog>
