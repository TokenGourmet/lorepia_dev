export interface CharacterSummary {
  id: string;
  name: string;
  initial: string;
  tagline: string;
  description: string;
  lastMessage: string;
  lastAt: Date;
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
    lastMessage: "“짧은 우화집도 함께 챙겨드릴게요. 어느 쪽이든, 오늘 밤은 혼자가 아니에요.”",
    lastAt: new Date(2026, 6, 18, 23, 44),
    scriptCount: 1,
  },
  {
    id: "kai",
    name: "카이",
    initial: "카",
    tagline: "별을 세는 등대지기",
    description:
      "육지에서 가장 먼 등대에서 혼자 별을 센다. 무전기 너머로만 대화할 수 있지만, 그의 목소리는 언제나 파도보다 가깝게 들린다.",
    lastMessage: "오늘은 유성이 셋. 하나는 네 몫으로 세어 뒀어.",
    lastAt: new Date(2026, 6, 19, 2, 17),
    scriptCount: 0,
  },
];

export function findSampleCharacter(id: string): CharacterSummary | undefined {
  return SAMPLE_CHARACTERS.find((character) => character.id === id);
}
