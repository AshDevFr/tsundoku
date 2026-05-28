import Link from "@docusaurus/Link";
import useDocusaurusContext from "@docusaurus/useDocusaurusContext";
import HomepageFeatures from "@site/src/components/HomepageFeatures";
import Heading from "@theme/Heading";
import Layout from "@theme/Layout";
import type { ReactNode } from "react";

import styles from "./index.module.css";

function HomepageHeader() {
  const { siteConfig } = useDocusaurusContext();
  return (
    <header className={styles.heroBanner}>
      <div className="container">
        <Heading as="h1" className={styles.heroTitle}>
          {siteConfig.title}
        </Heading>
        <p className={styles.heroSubtitle}>{siteConfig.tagline}</p>
        <p className={styles.heroBlurb}>
          Poll discovery sources, resolve releases against MangaBaka, and
          surface the manga you don't own yet. Self-hosted, single
          binary, Codex-friendly.
        </p>
        <div className={styles.buttons}>
          <Link className={styles.primaryButton} to="/docs/">
            Get Started
          </Link>
          <Link
            className={styles.secondaryButton}
            href="https://github.com/AshDevFr/tsundoku"
          >
            GitHub
          </Link>
        </div>
      </div>
    </header>
  );
}

function QuickLinks(): ReactNode {
  return (
    <section className={styles.quickLinks}>
      <div className="container">
        <div className="row">
          <div className="col col--4">
            <Link to="/docs/" className={styles.quickLink}>
              <span className={styles.quickLinkIcon}>📖</span>
              <span className={styles.quickLinkText}>Introduction</span>
            </Link>
          </div>
          <div className="col col--4">
            <Link
              href="https://github.com/AshDevFr/tsundoku#readme"
              className={styles.quickLink}
            >
              <span className={styles.quickLinkIcon}>⚙️</span>
              <span className={styles.quickLinkText}>Setup</span>
            </Link>
          </div>
          <div className="col col--4">
            <Link
              href="https://github.com/AshDevFr/tsundoku/issues"
              className={styles.quickLink}
            >
              <span className={styles.quickLinkIcon}>🐛</span>
              <span className={styles.quickLinkText}>Issues</span>
            </Link>
          </div>
        </div>
      </div>
    </section>
  );
}

export default function Home(): ReactNode {
  const { siteConfig } = useDocusaurusContext();
  return (
    <Layout
      title={`${siteConfig.title} · ${siteConfig.tagline}`}
      description="Manga discovery sidecar for Codex. Polls release sources, resolves to MangaBaka, surfaces what you don't own."
    >
      <HomepageHeader />
      <main>
        <QuickLinks />
        <HomepageFeatures />
      </main>
    </Layout>
  );
}
