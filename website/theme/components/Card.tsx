import type { ReactNode } from 'react';
import './cards.css';

interface CardProps {
  title: string;
  href: string;
  children?: ReactNode;
}

export default function Card({ title, href, children }: CardProps) {
  return (
    <a className="rune-cards__item" href={href}>
      <span className="rune-cards__title">{title}</span>
      <span className="rune-cards__body">{children}</span>
    </a>
  );
}
