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

function startOfDay(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

/* The centered thread separator, iMessage style: relative day words while
   they read naturally, calendar dates beyond, year only when it differs. */
export function formatThreadStamp(sentAt: Date, now: Date = new Date()): string {
  const dayDiff = Math.round(
    (startOfDay(now) - startOfDay(sentAt)) / 86_400_000,
  );
  const time = formatMessageTime(sentAt);
  if (dayDiff <= 0) {
    return `오늘 ${time}`;
  }
  if (dayDiff === 1) {
    return `어제 ${time}`;
  }
  const date = `${sentAt.getMonth() + 1}월 ${sentAt.getDate()}일`;
  if (sentAt.getFullYear() === now.getFullYear()) {
    return `${date} ${time}`;
  }
  return `${sentAt.getFullYear()}년 ${date} ${time}`;
}
