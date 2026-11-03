<script lang="ts">
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import { panel } from "../panel.svelte";
  import SettingsDialog from "./SettingsDialog.svelte";

  // アイテム説明のカードを出しておく秒数（範囲はサーバーと同じ）
  const RANGE = { min: 3, max: 30 };

  let seconds = $state(0);
  /** 開いたときの値。変えていなければ送らない */
  let opened = 0;
  const valid = $derived(Number.isFinite(seconds) && seconds >= RANGE.min && seconds <= RANGE.max);
</script>

<SettingsDialog
  trigger="数値で指定"
  title="アイテム説明を出しておく時間"
  description="1 枚のカードを出しておく秒数（{RANGE.min}〜{RANGE.max}）。出しているアイテムをもう一度話すと、最初から数え直します"
  {valid}
  onopen={() => (seconds = opened = panel.settings!.itemCards.holdSeconds)}
  onsave={async () => seconds === opened || panel.update({ itemCards: { holdSeconds: seconds } })}
>
  <Field.Field orientation="horizontal">
    <Field.Label for="item-hold" class="flex-1">出しておく時間（秒）</Field.Label>
    <Input
      id="item-hold"
      type="number"
      class="w-24"
      min={RANGE.min}
      max={RANGE.max}
      step="any"
      bind:value={seconds}
    />
  </Field.Field>
</SettingsDialog>
