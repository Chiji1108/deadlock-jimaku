<script lang="ts">
  // 使っているヒーロー。自分で選ぶか、試合から取ったもの（`panel.view.played`）に任せる（DESIGN.md 9.13）。
  // 試合ごとに変わるので、プロフィールではなくホームに置く
  import * as Card from "$lib/components/ui/card";
  import * as Select from "$lib/components/ui/select";
  import { entityName, glossary, loadGlossary } from "../../lib/glossary.svelte";
  import { panel } from "../panel.svelte";

  const s = $derived(panel.settings!);
  const AUTO = "auto";
  void loadGlossary();
  const heroName = (ref: string | undefined) => {
    const entity = ref ? glossary.entity(ref) : undefined;
    return entity ? entityName(entity, "ja") : undefined;
  };
  const heroes = $derived(
    glossary.heroes
      .map(([ref, entity]) => ({ ref, name: entityName(entity, "ja"), icon: entity.icon }))
      .toSorted((a, b) => a.name.localeCompare(b.name, "ja")),
  );
  const auto = $derived.by(() => {
    if (!s.rankAccount) return "自動（Steam のアカウントをつなぐと、試合から取ります）";
    return `自動（${heroName(panel.view?.played) ?? "まだ分かりません"}）`;
  });
</script>

<Card.Root>
  <Card.Header>
    <Card.Title>使っているヒーロー</Card.Title>
    <Card.Description>
      「ウルト使う」のように名前を言わなくても、このヒーローのスキルとして強調します
    </Card.Description>
  </Card.Header>
  <Card.Content class="flex flex-col gap-2">
    <Select.Root
      type="single"
      value={s.playing ?? AUTO}
      onValueChange={(v) => panel.update({ playing: v === AUTO ? "" : v })}
      onOpenChange={(open) => open && void loadGlossary()}
    >
      <Select.Trigger class="w-full">{heroName(s.playing) ?? auto}</Select.Trigger>
      <Select.Content>
        <Select.Item value={AUTO}>{auto}</Select.Item>
        {#each heroes as hero (hero.ref)}
          <Select.Item value={hero.ref}>
            {#if hero.icon}<img class="size-5 rounded" src={hero.icon} alt="" />{/if}
            {hero.name}
          </Select.Item>
        {/each}
      </Select.Content>
    </Select.Root>
    <p class="text-xs text-muted-foreground">
      用語を AI
      で判定するモード（おすすめ・Chrome+・クラウド・ローカル）で効きます。自動は、ランクの表示につないだ
      Steam
      のアカウントの試合から取ります。進行中の試合が分かるのは上位の試合だけなので、ふつうは前の試合のヒーローになります
    </p>
  </Card.Content>
</Card.Root>
