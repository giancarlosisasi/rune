import type { ReactNode } from 'react';
import './cards.css';

/** A grid of links into deeper pages. Registered globally, so pages need no import. */
export default function Cards({ children }: { children?: ReactNode }) {
  return <div className="rune-cards">{children}</div>;
}
