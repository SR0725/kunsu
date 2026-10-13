import { useCallback, useRef, type ButtonHTMLAttributes, type PointerEvent } from 'react'

export function SpecularButton({
  className = '',
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement>) {
  const ref = useRef<HTMLButtonElement>(null)
  const steer = useCallback((event: PointerEvent<HTMLButtonElement>) => {
    const rect = event.currentTarget.getBoundingClientRect()
    const x = ((event.clientX - rect.left) / rect.width) * 100
    const y = ((event.clientY - rect.top) / rect.height) * 100
    ref.current?.style.setProperty('--shine-x', `${x}%`)
    ref.current?.style.setProperty('--shine-y', `${y}%`)
  }, [])

  return (
    <button
      {...props}
      ref={ref}
      onPointerMove={steer}
      className={`specular-button ${className}`}
    >
      <span className="specular-button__label">{children}</span>
    </button>
  )
}
