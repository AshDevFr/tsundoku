import {
  Anchor,
  Badge,
  Container,
  Group,
  AppShell as MantineAppShell,
  type MantineColor,
  Text,
  Title,
} from "@mantine/core";
import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { useStats } from "@/api/queries";

export function AppShell({ children }: { children: ReactNode }) {
  const stats = useStats();
  const reviewCount =
    (stats.data?.releases.unresolved ?? 0) +
    (stats.data?.releases.ambiguous ?? 0) +
    (stats.data?.releases.reviewPending ?? 0);

  return (
    <MantineAppShell header={{ height: 56 }} padding={0}>
      <MantineAppShell.Header>
        <Container size="xl" h="100%">
          <Group h="100%" justify="space-between" align="center" wrap="nowrap">
            <Anchor component={Link} to="/" underline="never" c="inherit">
              <Group gap="xs" align="baseline">
                <Title order={3}>tsundoku</Title>
                <Text size="xs" c="dimmed">
                  discovery
                </Text>
              </Group>
            </Anchor>
            <Group gap="md" align="center" wrap="nowrap">
              {typeof stats.data?.series === "number" && (
                <Text size="sm" c="dimmed" component="span">
                  {stats.data.series} series
                </Text>
              )}
              <Badge
                component={Link}
                to="/review"
                color={(reviewCount > 0 ? "orange" : "gray") as MantineColor}
                variant={reviewCount > 0 ? "light" : "default"}
                radius="sm"
                style={{ cursor: "pointer", textDecoration: "none" }}
                aria-label={`Review queue (${reviewCount} pending)`}
              >
                {reviewCount > 0 ? `${reviewCount} to review` : "review"}
              </Badge>
              {stats.data?.activeProvider && (
                <Badge variant="default" radius="sm">
                  {stats.data.activeProvider}
                </Badge>
              )}
            </Group>
          </Group>
        </Container>
      </MantineAppShell.Header>
      <MantineAppShell.Main>{children}</MantineAppShell.Main>
    </MantineAppShell>
  );
}
