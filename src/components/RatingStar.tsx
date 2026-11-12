/**
 * One star of a rating, lit or not.
 *
 * Drawn, not typed: none of the fonts in our stack has ★ or ☆, so each
 * platform used to substitute its own and the stars came out a different size
 * everywhere. The caller's class sets the height; the width follows the
 * glyph's proportions.
 *
 * The colour is Energy's own, never the theme's (docs/design/tokens.md): lit
 * stars in the track's level colour, unlit ones in the unset colour, and a
 * star under the pointer in a button shows the level it would set, the same
 * colours the new list's pips use.
 */
import type { CSSProperties } from "react";
import { StarEmptyIcon, StarLitIcon } from "./icons";
import styles from "./RatingStar.module.css";

const level = (n: number): string => `var(--c-energy-${Math.min(5, Math.max(0, Math.round(n)))})`;

export function RatingStar({ star, rating, className }: { star: number; rating: number; className?: string | undefined }) {
  const lit = star <= rating;
  const Icon = lit ? StarLitIcon : StarEmptyIcon;
  const colours = { "--c-star-level": level(rating), "--c-star-own": level(star) } as CSSProperties;
  return (
    <Icon
      className={className ? `${styles.star} ${className}` : styles.star}
      data-lit={lit || undefined}
      style={colours}
    />
  );
}
