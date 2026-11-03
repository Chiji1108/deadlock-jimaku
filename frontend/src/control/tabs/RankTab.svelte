<script lang="ts">
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import * as Select from "$lib/components/ui/select";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import type { Numerals } from "../../generated/Numerals";
  import type { RankContent } from "../../generated/RankContent";
  import type { Region } from "../../generated/Region";
  import { REGIONS } from "../../lib/regions";
  import AddToObs from "../parts/AddToObs.svelte";
  import AnchorPicker from "../parts/AnchorPicker.svelte";
  import OverlayPreview from "../parts/OverlayPreview.svelte";
  import { RANK_SIZE } from "../../lib/overlay-size";
  import { overlayUrl, panel } from "../panel.svelte";

  const CONTENTS: { value: RankContent; label: string }[] = [
    { value: "full", label: "アイコン・名前・ポイント" },
    { value: "name", label: "アイコンと名前" },
    { value: "icon", label: "アイコンだけ" },
  ];
  const regions = Object.entries(REGIONS) as [Region, { ja: string }][];
  const NO_REGION = "";

  const s = $derived(panel.settings!);
  // ランク名を取ってくる言語（話す言語・英語・翻訳先）から選ぶ
  const languages = $derived([...new Set([s.micLanguage, "en", ...s.translateTo])]);
  /** 選べる言語にないもの（翻訳先から外した）は、サーバーと同じく話す言語に合わせる */
  const FOLLOW = "";
  const language = $derived(
    s.rankLanguage && languages.includes(s.rankLanguage) ? s.rankLanguage : FOLLOW,
  );

  const saved = () => panel.settings?.rankAccount?.toString() ?? "";
  let account = $state(saved());

  /** サーバーは SteamID64 も 32bit の ID に直して保存するので、保存した値を入力欄に戻す */
  async function saveAccount() {
    // 失敗したら直せるよう、入力は残す
    if (await panel.update({ rankAccount: account })) account = saved();
  }
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
          void saveAccount();
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
            <Button type="submit" disabled={account === saved()}>保存</Button>
          </div>
          <Field.Description>
            SteamID64（7656… で始まる 17 桁）か、フレンドコード。空にすると表示しません
          </Field.Description>
        </Field.Field>
      </form>

      <Field.Field>
        <Field.Label>出すもの</Field.Label>
        <ToggleGroup.Root
          type="single"
          variant="outline"
          class="w-full"
          value={s.rankContent}
          onValueChange={(v) => v && panel.update({ rankContent: v as RankContent })}
        >
          {#each CONTENTS as c (c.value)}
            <ToggleGroup.Item value={c.value} class="flex-1">{c.label}</ToggleGroup.Item>
          {/each}
        </ToggleGroup.Root>
      </Field.Field>

      <Field.Field>
        <Field.Label>サブランクの書き方</Field.Label>
        <ToggleGroup.Root
          type="single"
          variant="outline"
          class="w-full"
          value={s.rankNumerals}
          onValueChange={(v) => v && panel.update({ rankNumerals: v as Numerals })}
        >
          <ToggleGroup.Item value="roman" class="flex-1">III（ゲームと同じ）</ToggleGroup.Item>
          <ToggleGroup.Item value="arabic" class="flex-1">3</ToggleGroup.Item>
        </ToggleGroup.Root>
      </Field.Field>

      <Field.Field>
        <Field.Label>位置</Field.Label>
        <AnchorPicker
          label="ランクの位置"
          value={s.rankAnchor}
          onchange={(rankAnchor) => panel.update({ rankAnchor })}
        />
        <Field.Description>OBS のソースの箱の中で、ランクを寄せる位置です</Field.Description>
      </Field.Field>

      <Field.Field>
        <Field.Label>エターナスの順位</Field.Label>
        <Select.Root
          type="single"
          value={s.rankRegion ?? NO_REGION}
          onValueChange={(v) => panel.update({ rankRegion: v })}
        >
          <Select.Trigger class="w-full">
            {s.rankRegion ? REGIONS[s.rankRegion].ja : "出さない"}
          </Select.Trigger>
          <Select.Content>
            <Select.Item value={NO_REGION}>出さない</Select.Item>
            {#each regions as [value, name] (value)}
              <Select.Item {value}>{name.ja}のリーダーボード</Select.Item>
            {/each}
          </Select.Content>
        </Select.Root>
        <Field.Description>
          エターナスのときだけ、ポイントの代わりにリーダーボードの順位（例: アジア
          123位）を出します。更新は 1
          時間ごとです。載っていないときや、同じ名前のプレイヤーがいて見分けられないときはポイントを出します
        </Field.Description>
      </Field.Field>

      <Field.Field>
        <Field.Label>ランク名の言語</Field.Label>
        <Select.Root
          type="single"
          value={language}
          onValueChange={(v) => panel.update({ rankLanguage: v })}
        >
          <Select.Trigger class="w-full">
            {language === FOLLOW
              ? `話す言語に合わせる（${panel.label(s.micLanguage)}）`
              : panel.label(language)}
          </Select.Trigger>
          <Select.Content>
            <Select.Item value={FOLLOW}
              >話す言語に合わせる（{panel.label(s.micLanguage)}）</Select.Item
            >
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
        大きさを変えるときは、ソースを引き伸ばさずにプロパティの幅と高さを同じ割合で変えてください。中身がくっきりしたまま大きくなります
      </Card.Description>
    </Card.Header>
    <Card.Content>
      <AddToObs url={overlayUrl("rank")} name="ランク" size={RANK_SIZE} />
    </Card.Content>
  </Card.Root>
</div>
