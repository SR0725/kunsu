import type { ComponentPropsWithRef } from 'react'
import type { IconKey } from '../status'
import { Icon } from './Icon'

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger'
export type ButtonSize = 'sm' | 'md' | 'lg'

export type ButtonProps = ComponentPropsWithRef<'button'> & {
  variant?: ButtonVariant
  /** sm 28 px（表格內）、md 32 px、lg 36 px（頁面主要動作）。 */
  size?: ButtonSize
  icon?: IconKey
  /** 圖示放在文字後面（例如「進入 Kunsu →」）。 */
  trailingIcon?: IconKey
}

/** DESIGN「Buttons」：主要黑底、次要白底細框、ghost 無框、danger 紅字淺紅底；8 px 圓角。 */
export function Button({ variant = 'secondary', size = 'md', icon, trailingIcon, className, children, type = 'button', ...rest }: ButtonProps) {
  return (
    <button type={type} className={className ? `k-btn ${className}` : 'k-btn'} data-variant={variant} data-size={size} {...rest}>
      {icon && <Icon name={icon} size={size === 'sm' ? 14 : 16} />}
      {children}
      {trailingIcon && <Icon name={trailingIcon} size={size === 'sm' ? 14 : 16} />}
    </button>
  )
}
