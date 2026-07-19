export type DayPeriod = "새벽" | "아침" | "낮" | "저녁" | "밤";

export function dayPeriod(hour: number): DayPeriod {
  if (!Number.isInteger(hour) || hour < 0 || hour > 23) {
    throw new RangeError(`invalid hour: ${hour}`);
  }
  if (hour < 6) {
    return "새벽";
  }
  if (hour < 11) {
    return "아침";
  }
  if (hour < 17) {
    return "낮";
  }
  if (hour < 21) {
    return "저녁";
  }
  return "밤";
}

export function formatMessageTime(sentAt: Date): string {
  const hour = sentAt.getHours();
  const minute = sentAt.getMinutes();
  const clockHour = hour % 12 === 0 ? 12 : hour % 12;
  return `${dayPeriod(hour)} ${clockHour}:${String(minute).padStart(2, "0")}`;
}
