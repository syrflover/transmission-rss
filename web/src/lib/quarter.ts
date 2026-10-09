/** "2026년 4분기": the name of an anime quarter as every screen shows it. */
export function quarterName(quarter: { year: number; number: number }): string {
  return `${quarter.year}년 ${quarter.number}분기`;
}
