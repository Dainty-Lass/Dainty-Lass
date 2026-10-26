import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@workspace/ui/components/select"

export interface Option<T> {
  value: T
  label: string
  lang?: string
}

// A drop-down of fixed options; the backend answers each change with a new snapshot
export function Choice<T extends string | number>({
  id,
  value,
  options,
  onChange,
  disabled,
}: {
  id: string
  value: T
  options: Option<T>[]
  onChange(value: T): void
  disabled?: boolean
}) {
  return (
    <Select
      items={options}
      value={value}
      disabled={disabled}
      onValueChange={(next) => {
        if (next !== null) onChange(next as T)
      }}
    >
      <SelectTrigger id={id} className="min-w-48">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {options.map((o) => (
          <SelectItem key={String(o.value)} value={o.value} lang={o.lang}>
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
