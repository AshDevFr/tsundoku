import Heading from "@theme/Heading";
import clsx from "clsx";
import type { ReactNode } from "react";
import styles from "./styles.module.css";

type FeatureItem = {
  title: string;
  icon: string;
  description: ReactNode;
};

const FeatureList: FeatureItem[] = [
  {
    title: "Multi-source discovery",
    icon: "📡",
    description: (
      <>
        Plug in any number of release feeds. v1 ships a{" "}
        <strong>Nyaa</strong> source with RSS + per-post enrichment; the{" "}
        <code>DiscoverySource</code> trait is the contract for adding more.
      </>
    ),
  },
  {
    title: "Resolves to MangaBaka",
    icon: "🔗",
    description: (
      <>
        Every release runs through a four-step pipeline: known external ID
        → foreign-ID lookup → fuzzy-title search → format validation.
        Confident matches auto-link; ambiguous ones land in the review
        queue.
      </>
    ),
  },
  {
    title: "Review queue, not a black box",
    icon: "🪄",
    description: (
      <>
        Releases that don't auto-resolve get a card with cleaned search
        queries, applied cleanup rules, candidate covers, and one-click
        provider-search to link them by hand.
      </>
    ),
  },
  {
    title: "Built in Rust",
    icon: "⚡",
    description: (
      <>
        Single binary, single SQLite file. axum + sea-orm + tokio. The
        React SPA is embedded via <code>rust-embed</code> behind a feature
        flag.
      </>
    ),
  },
  {
    title: "Codex-friendly",
    icon: "🤝",
    description: (
      <>
        Designed to run alongside <strong>Codex</strong>. tsundoku
        surfaces what you don't own; Codex tracks what you do. No shared
        database — clean HTTP boundary between the two.
      </>
    ),
  },
  {
    title: "Operator-first",
    icon: "🛠️",
    description: (
      <>
        Admin page surfaces per-source config, last-poll markers, manual
        triggers, and a metrics tab with resolution-outcome breakdowns +
        review-queue depth over time.
      </>
    ),
  },
];

function Feature({ title, icon, description }: FeatureItem) {
  return (
    <div className={clsx("col col--4")}>
      <div className={styles.featureCard}>
        <div className={styles.featureIcon}>{icon}</div>
        <Heading as="h3" className={styles.featureTitle}>
          {title}
        </Heading>
        <p className={styles.featureDescription}>{description}</p>
      </div>
    </div>
  );
}

export default function HomepageFeatures(): ReactNode {
  return (
    <section className={styles.features}>
      <div className="container">
        <div className="row">
          {FeatureList.map((props) => (
            <Feature key={props.title} {...props} />
          ))}
        </div>
      </div>
    </section>
  );
}
