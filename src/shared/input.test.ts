import { describe, expect, it } from 'vitest';
import { inputWithinLimits } from './input';

describe('translation input limits', () => {
  it('counts UTF-16 units, accepts the boundary, and never truncates input', () => {
    expect(inputWithinLimits('🌊'.repeat(2_500))).toBe(true);
    expect(inputWithinLimits('🌊'.repeat(2_501))).toBe(false);
    expect(inputWithinLimits(Array(20).fill('a'.repeat(4_999)).join('\n'))).toBe(true);
    expect(inputWithinLimits(Array(21).fill('a'.repeat(4_999)).join('\n'))).toBe(false);
  });

  it('handles CRLF as one line boundary and includes the trailing blank line', () => {
    expect(inputWithinLimits('a\r\n'.repeat(1_999))).toBe(true);
    expect(inputWithinLimits('a\r\n'.repeat(2_000))).toBe(false);
  });
});
