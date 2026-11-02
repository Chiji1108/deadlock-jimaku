<script lang="ts">
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import * as Select from "$lib/components/ui/select";
  import CopyUrl from "../parts/CopyUrl.svelte";
  import OverlayPreview from "../parts/OverlayPreview.svelte";
  import { overlayUrl, panel } from "../panel.svelte";

  /** OBS のブラウザソースの大きさ */
  const RANK_SIZE: [number, number] = [480, 140];

  const s = $derived(panel.settings!);
  // ランク名を取ってくる言語（話す言語・英語・翻訳先）から選ぶ
  const languages = $derived([...new Set([s.micLanguage, "en", ...s.translateTo])]);
  const language = $derived(s.rankLanguage ?? s.micLanguage);

  let account = $state(panel.settings?.rankAccount?.toString() ?? "");
</script>

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title>ランクオーバーレイ</Card.Title>
      <Card.Description>
        ランクのアイコン・名前・ポイントを出します。試合の結果が反映されるまで少しかかります
      </Card.Description>
    </Card.Header>
    <Card.Content class="flex flex-col gap-6">
      <OverlayPreview
        src="/overlay/rank.html"
        title="ランクオーバーレイのプレビュー"
        size={RANK_SIZE}
      />

      <form
        onsubmit={(e) => {
          e.preventDefault();
          void panel.update({ rankAccount: account });
        }}
      >
        <Field.Field>
          <Field.Label for="account">Steam ID</Field.Label>
          <div class="flex gap-2">
            <Input
              id="account"
              bind:value={account}
              placeholder="76561198…"
              inputmode="numeric"
              autocomplete="off"
            />
            <Button type="submit" disabled={account === (s.rankAccount?.toString() ?? "")}>
              保存
            </Button>
          </div>
          <Field.Description>
            SteamID64（7656… で始まる 17 桁）か、フレンドコード。空にすると表示しません
          </Field.Description>
        </Field.Field>
      </form>

      <Field.Field>
        <Field.Label>ランク名の言語</Field.Label>
        <Select.Root
          type="single"
          value={language}
          onValueChange={(v) => panel.update({ rankLanguage: v })}
        >
          <Select.Trigger class="w-full">{panel.label(language)}</Select.Trigger>
          <Select.Content>
            {#each languages as code (code)}
              <Select.Item value={code}>{panel.label(code)}</Select.Item>
            {/each}
          </Select.Content>
        </Select.Root>
      </Field.Field>
    </Card.Content>
  </Card.Root>

  <Card.Root>
    <Card.Header>
      <Card.Title>OBS に追加する</Card.Title>
      <Card.Description>
        ソースの「＋」→「ブラウザ」で、この URL を貼り付けてください（幅 {RANK_SIZE[0]}・高さ
        {RANK_SIZE[1]}）
      </Card.Description>
    </Card.Header>
    <Card.Content>
      <CopyUrl url={overlayUrl("rank")} />
    </Card.Content>
  </Card.Root>
</div>
