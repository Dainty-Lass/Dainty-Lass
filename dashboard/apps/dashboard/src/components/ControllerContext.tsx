"use client";

import { Link } from "@astryxdesign/core/Link";
import { Text } from "@astryxdesign/core/Text";
import { createContext, type ReactNode, use } from "react";

interface Value {
  names: Record<string, string>;
  /** True when rows from several controllers share a list, so each needs its controller named. */
  fleetWide: boolean;
}

const Context = createContext<Value>({ names: {}, fleetWide: false });

export function ControllerProvider({ value, children }: { value: Value; children: ReactNode }) {
  return <Context value={value}>{children}</Context>;
}

export function useControllers(): Value {
  return use(Context);
}

/** A row's controller, linked; a removed controller still shows its id. */
export function ControllerLabel({ id }: { id: string }) {
  const { names } = useControllers();
  return names[id] ? (
    <Link href={`/controllers/${id}`}>{names[id]}</Link>
  ) : (
    <Text type="supporting">{id.slice(0, 8)}</Text>
  );
}
