import { Heading } from "@astryxdesign/core/Heading";
import { Layout, LayoutContent, LayoutHeader } from "@astryxdesign/core/Layout";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import type { ReactNode } from "react";

/** Every dashboard page's frame: a title row with its actions, then content on the body colour. */
export function Page({
  title,
  description,
  actions,
  breadcrumbs,
  tabs,
  children,
  width = 1280,
}: {
  title: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  breadcrumbs?: ReactNode;
  tabs?: ReactNode;
  children: ReactNode;
  width?: number;
}) {
  return (
    <Layout
      contentWidth={width}
      header={
        <LayoutHeader>
          <Stack gap={3} paddingInline={6} paddingBlockStart={5} paddingBlockEnd={tabs ? 0 : 3}>
            {breadcrumbs}
            <Stack direction="horizontal" gap={4} justify="between" align="end" wrap="wrap">
              <Stack gap={1}>
                <Heading level={1}>{title}</Heading>
                {description && <Text type="supporting">{description}</Text>}
              </Stack>
              {actions && (
                <Stack direction="horizontal" gap={2} align="center" wrap="wrap">
                  {actions}
                </Stack>
              )}
            </Stack>
            {tabs}
          </Stack>
        </LayoutHeader>
      }
      content={
        <LayoutContent padding={6}>
          <Stack gap={5}>{children}</Stack>
        </LayoutContent>
      }
    />
  );
}
