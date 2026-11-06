<script lang="ts" module>
  /** 期限が過ぎていなくて、1 日の上限が正の数か空欄（既定の額） */
  export const expiryPassed = (expiresAt: number | null, now: number) =>
    expiresAt !== null && expiresAt <= now;
  export const validCode = (expiresAt: number | null, limit: number | null, now: number) =>
    !expiryPassed(expiresAt, now) && (limit === null || limit > 0);
</script>

<script lang="ts">
  // ギフトコードのメモ・期限・1 日の上限。作るときと変えるときで同じ欄を使う
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import { dateTime, panel } from "../panel.svelte";
  import DailyLimitField from "./DailyLimitField.svelte";
  import ExpiryPicker from "./ExpiryPicker.svelte";

  let {
    id,
    memo = $bindable(),
    expiresAt = $bindable(),
    dailyLimit = $bindable(),
    placeholder,
  }: {
    id: string;
    memo: string;
    expiresAt: number | null;
    dailyLimit: number | null;
    placeholder?: string;
  } = $props();
</script>

<Field.Field>
  <Field.Label for="{id}-memo">メモ（自分だけに見えます）</Field.Label>
  <Input id="{id}-memo" bind:value={memo} maxlength={100} {placeholder} />
</Field.Field>
<Field.Field>
  <Field.Label>期限</Field.Label>
  <ExpiryPicker bind:value={expiresAt} />
  <Field.Description>
    {#if expiryPassed(expiresAt, panel.now)}
      <span class="text-destructive">この期限はもう過ぎています。選び直してください</span>
    {:else}
      {expiresAt === null
        ? "期限なしで使えます。あとから変えられます"
        : `${dateTime(expiresAt)} まで使えます。あとから変えられます。期限が過ぎても、出している字幕は止まりません`}
    {/if}
  </Field.Description>
</Field.Field>
<DailyLimitField id="{id}-limit" bind:value={dailyLimit} />
