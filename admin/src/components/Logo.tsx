import { cn } from '@/utils';

type LogoProps = {
  className?: string;
  alt?: string;
};

export function Logo({ className, alt = 'Pertisk' }: LogoProps) {
  return <img src="/logo.png" alt={alt} className={cn('object-contain', className)} draggable={false} />;
}
