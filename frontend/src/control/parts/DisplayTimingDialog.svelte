<script lang="ts">
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import type { SubtitleDisplay } from "../../generated/SubtitleDisplay";
  import type { SubtitleDisplayPatch } from "../../generated/SubtitleDisplayPatch";
  import { panel } from "../panel.svelte";
  import SettingsDialog from "./SettingsDialog.svelte";

  // 字幕をどれだけ出しておくかを決める秒数（範囲はサーバーと同じ）
  const FIELDS = [
    {
      key: "holdSeconds",
      label: "消すまでの時間",
      hint: "翻訳が届いてから消すまでの時間です。既定は 6 秒",
      min: 2,
      max: 30,
    },
    {
      key: "perCharSeconds",
      label: "長い文に足す時間（1 文字あたり）",
      hint: "長い文ほど長く出します。既定は 0.15 秒",
      min: 0,
      max: 1,
    },
    {
      key: "minReadSeconds",
      label: "翻訳を読む時間",
      hint: "同時に出す数を超えても、翻訳が届いてからこの間は字幕を残します。既定は 3 秒",
      min: 1,
      max: 30,
    },
    {
      key: "maxWaitSeconds",
      label: "次の字幕を待たせる上限",
      hint: "読み終わっていない字幕があっても、この時間で次の字幕に切り替えます（「翻訳を読む時間」より優先します）。0 ならすぐ切り替えます。既定は 6 秒",
      min: 0,
      max: 30,
    },
  ] as const;
  type Values = Pick<SubtitleDisplay, (typeof FIELDS)[number]["key"]>;

  const read = (): Values => {
    const { holdSeconds, perCharSeconds, minReadSeconds, maxWaitSeconds } =
      panel.settings!.subtitleDisplay;
    return { holdSeconds, perCharSeconds, minReadSeconds, maxWaitSeconds };
  };

  /** 開くたびに open で今の設定から始める */
  let values = $state<Values>(read());
  /** 開いたときの値。変えた項目だけを送る（ほかの画面で変えた値を戻さない） */
  let opened = read();
  const valid = $derived(
    FIELDS.every(({ key, min, max }) => {
      const n = values[key];
      return Number.isFinite(n) && n >= min && n <= max;
    }),
  );

  function open() {
    opened = read();
    values = read();
  }

  async function save() {
    const patch: SubtitleDisplayPatch = {};
    for (const { key } of FIELDS) if (values[key] !== opened[key]) patch[key] = values[key];
    return Object.keys(patch).length === 0 || panel.update({ subtitleDisplay: patch });
  }
</script>

<SettingsDialog
  trigger="詳しく設定"
  title="字幕の表示時間"
  description="同時に出す数を超えたら、読み終えた字幕から消します。読み終えた字幕がなければ、次の字幕を待たせます"
  {valid}
  onopen={open}
  onsave={save}
>
  <Field.Group>
    {#each FIELDS as f (f.key)}
      <Field.Field>
        <div class="flex items-center gap-4">
          <Field.Label for={f.key} class="flex-1">{f.label}（秒）</Field.Label>
          <Input
            id={f.key}
            type="number"
            class="w-24"
            min={f.min}
            max={f.max}
            step="any"
            bind:value={values[f.key]}
          />
        </div>
        <Field.Description>{f.hint}</Field.Description>
      </Field.Field>
    {/each}
  </Field.Group>
</SettingsDialog>
