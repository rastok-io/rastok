# Rastok Trademark Policy

**Status:** first draft. May be refined as the project matures.
Significant changes will go through a public RFC.

This policy describes how the Rastok name, logo, and related marks may
be used. It follows common open-source trademark-policy patterns.

> **The Rastok trademark policy does not limit any rights granted by the
> [Apache License 2.0](LICENSE).** You are free to use, copy, modify,
> fork, build, run, and redistribute the Rastok software under the
> Apache License. This trademark policy is about *names*, not *code*.

## 1. Marks covered

This policy covers:

- The word mark **"Rastok"**.
- Any future Rastok logos and visual identity.
- Any compound mark explicitly designated by the project as an official
  artifact, service, or program.

## 2. Allowed without permission

You may freely, and without asking us first:

- **Refer to the project** in documentation, tutorials, blog posts, talks,
  social media, books, courses, benchmarks, comparisons, and reviews.
  *"This is a tutorial on Rastok."* *"We benchmarked Rastok against
  Apache NiFi."* *"Rastok is great for X."*

- **State that your software works with Rastok**: *"FooApp is compatible
  with Rastok."* *"BarConnector integrates with Rastok."* *"BazSDK is a
  client for Rastok."*

- **Build and publish plugins, connectors, integrations** that work with
  Rastok, including under names such as `rastok-<your-thing>`,
  `<your-thing>-for-rastok`, `<your-thing>-rastok-connector`, provided
  your use does not imply that your work is an official Rastok product,
  certified, or endorsed.

- **Fork, modify, and redistribute** the Rastok code under the Apache
  License 2.0. You may describe your distribution as "derived from
  Rastok" or "based on Rastok", but you must give your modified
  distribution its own name.

- **Provide consulting, training, or hosting services** that involve
  Rastok, and say so. *"We offer Rastok hosting."* *"FooConsulting helps
  teams deploy Rastok."*

## 3. Uses that require permission

You must obtain written permission from the Rastok project to:

- Use the Rastok name or logo in your company name, product name, or
  domain name in a way that implies an official relationship — e.g., a
  product called *"Official Rastok Pro"* or a domain like
  `rastok-official.example`.

- Use a compound mark that the project has explicitly designated as
  official — for example, future program names such as *"Rastok
  Verified"* or *"Official Rastok Distribution"*. Designated marks, if
  and when any exist, will be listed in this file; they are reserved
  for artifacts, services, or programs approved by the Rastok project.

- Distribute a modified version of Rastok under the name *"Rastok"* (as
  opposed to a different name with a *"based on Rastok"* description).

- Use the Rastok logo in a product user interface, marketing material,
  or merchandise.

- Organize an event under the Rastok name (e.g., *"RastokConf"*) in a
  way that suggests official sponsorship.

## 4. Uses that are prohibited

The following uses are not permitted, with or without permission:

- Implying false endorsement, certification, affiliation, or partnership.

- Registering domain names, package names, social-media handles, or
  trademarks that contain *"Rastok"* in a way that creates confusion
  with the official project. Specifically, names such as `rastokcloud`,
  `rastok-official`, `realrastok`, or close phonetic variants used in
  the same field are not permitted.

- Shipping a modified binary as *"Official Rastok"*, *"Rastok Build"*, or
  presenting a fork as if it were the canonical project.

## 5. Modified distributions and forks

You may fork, modify, and redistribute Rastok under the Apache License
2.0. When you do, give your distribution its own clear name:

> Good: *"FooFlow, derived from Rastok."*
>
> Good: *"FooFlow — a Rastok-compatible runtime."*
>
> Not OK: *"Rastok by FooCorp"* (uses "Rastok" as the product name).
>
> Not OK: *"Official FooCorp Rastok Distribution"* (implies endorsement).

## 6. Examples

Allowed without asking:

- *"A Rastok plugin for ingesting Slack messages."*
- *"FooBank uses Rastok in production."* (factual statement)
- *"Tutorial: building your first Rastok graph."*
- A blog post comparing Rastok to LangGraph, NiFi, or Temporal.
- A package named `rastok-postgres-connector` on crates.io or npm,
  clearly marked as community-maintained in its description.
- A talk titled *"Rastok in production: lessons learned"*.

Requires permission:

- A hosted service sold under *"Rastok"* as its product name.
- A startup called *"Rastok Corp"*.
- A product page leading with the Rastok logo as if owned.

Prohibited:

- A package named `rastok-official` on crates.io while not being official.
- A domain `rastok-cloud.com` redirecting to a competing service.
- A modified binary distributed as *"Rastok 1.0"*.

## 7. Permission requests

For uses that require permission, open an issue tagged `trademark` or
contact the maintainer through the public contact address in their
profile on the canonical repository.

We aim to respond within a reasonable timeframe. We generally approve
community uses that do not create confusion.

## 8. Policy evolution

This policy may evolve. Major changes will go through a public RFC with
a notice period before they take effect. Uses that were conformant under
a previous version of the policy will not be retroactively prohibited
without a reasonable transition period.
