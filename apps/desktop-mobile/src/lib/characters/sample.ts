export interface CharacterSummary {
  id: string;
  name: string;
  initial: string;
  tagline: string;
  description: string;
  scriptCount: number;
}

export const SAMPLE_CHARACTERS: CharacterSummary[] = [
  {
    id: "seraphine",
    name: "세라핀",
    initial: "세",
    tagline: "달빛 서고의 사서",
    description:
      "밤에만 문을 여는 서고를 지키는 사서. 찾는 책이 없는 손님에게도 언제나 한 권을 골라 준다. 그녀가 고른 책은 이상하게도, 그날 밤 그 사람에게 꼭 필요한 이야기다.",
    scriptCount: 1,
  },
  {
    id: "kai",
    name: "카이",
    initial: "카",
    tagline: "별을 세는 등대지기",
    description:
      "육지에서 가장 먼 등대에서 혼자 별을 센다. 무전기 너머로만 대화할 수 있지만, 그의 목소리는 언제나 파도보다 가깝게 들린다.",
    scriptCount: 0,
  },
  {
    id: "mira",
    name: "미라",
    initial: "미",
    tagline: "잠든 도시의 지도 제작자",
    description:
      "사람들이 잠든 사이 골목을 걸으며 매일 달라지는 도시의 지도를 그린다. 지도 가장자리에는 길보다 그날 만난 사람들의 이야기가 더 많이 적혀 있다.",
    scriptCount: 0,
  },
  {
    id: "yoonseul",
    name: "윤슬",
    initial: "윤",
    tagline: "바다의 우편배달부",
    description:
      "주소 없이 파도에 띄운 편지를 찾아 주인에게 전해 준다. 답장을 기다리지 않는다고 말하지만, 늘 빈 우편가방 하나를 남겨 둔다.",
    scriptCount: 1,
  },
  {
    id: "roen",
    name: "로엔",
    initial: "로",
    tagline: "기억을 수선하는 재봉사",
    description:
      "해진 옷과 함께 흐릿해진 기억을 꿰매는 작은 수선실의 주인. 완벽하게 고치기보다 오래 간직할 수 있게 만드는 일을 좋아한다.",
    scriptCount: 0,
  },
  {
    id: "adel",
    name: "아델",
    initial: "아",
    tagline: "마지막 열차의 차장",
    description:
      "자정 이후 단 한 번 운행하는 열차의 차장. 승객이 말하지 않아도 내려야 할 역을 알고 있지만 목적지를 재촉하는 법은 없다.",
    scriptCount: 1,
  },
  {
    id: "noah",
    name: "노아",
    initial: "노",
    tagline: "비 오는 날의 라디오 DJ",
    description:
      "비가 내리는 날에만 들리는 심야 방송을 진행한다. 사연의 주인을 묻지 않고 지금 필요한 노래 한 곡을 골라 준다.",
    scriptCount: 0,
  },
  {
    id: "isol",
    name: "이솔",
    initial: "이",
    tagline: "꿈을 기록하는 천문학자",
    description:
      "별의 움직임과 사람들이 잊어버린 꿈 사이의 규칙을 연구한다. 관측 일지에는 숫자보다 다정한 안부가 더 자주 등장한다.",
    scriptCount: 0,
  },
];

export function findSampleCharacter(id: string): CharacterSummary | undefined {
  return SAMPLE_CHARACTERS.find((character) => character.id === id);
}

export function characterChatTitle(name: string): string {
  const last = name.codePointAt(name.length - 1);
  const hasFinalConsonant =
    last !== undefined &&
    last >= 0xac00 &&
    last <= 0xd7a3 &&
    (last - 0xac00) % 28 !== 0;
  return `${name}${hasFinalConsonant ? "과" : "와"}의 대화`;
}
