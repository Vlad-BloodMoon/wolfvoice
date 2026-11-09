# Start your own WolfVoice development from BloodMoon v0.2.0-bloodmoon.4

This branch is a **development baseline** anchored to the source code of the [BloodMoon v0.2.0-bloodmoon.4 release](https://github.com/Vlad-BloodMoon/wolfvoice/releases/tag/v0.2.0-bloodmoon.4).

It is intended for developers who want to experiment, maintain their own changes, or publish a derivative fork. It is **not** the upstream WolfVoice project and does not automatically follow future changes on the `main` branch.

## Fork and start developing

1. Open [Vlad-BloodMoon/wolfvoice](https://github.com/Vlad-BloodMoon/wolfvoice).
2. Click **Fork** and create your own repository. If GitHub shows **Copy the main branch only**, **uncheck it** to include this baseline branch.
3. Clone your fork, then check out the baseline:

```bash
git clone https://github.com/YOUR-USERNAME/wolfvoice.git
cd wolfvoice
git switch bloodmoon-v0.2.0-base
git switch -c my-wolfvoice-development
git push -u origin my-wolfvoice-development
```

4. Make changes on your own branch. Keep the baseline for comparison.

Alternatively, if you already cloned this repo:

```bash
git fetch origin
git switch -c my-wolfvoice-development origin/bloodmoon-v0.2.0-base
```

## Pin to the exact original release

This baseline branch was **created from the Git tag** `v0.2.0-bloodmoon.4`. This guide is an additional commit, so if you need the **exact release snapshot**, check out the tag rather than the branch:

```bash
git fetch --tags
git switch -c my-exact-release v0.2.0-bloodmoon.4
```

## Release downloads and documentation

- [Release page and downloadable assets](https://github.com/Vlad-BloodMoon/wolfvoice/releases/tag/v0.2.0-bloodmoon.4)
- [Original WolfVoice project](https://github.com/intelligentwolf/wolfvoice)
- [BloodMoon repository main branch](https://github.com/Vlad-BloodMoon/wolfvoice/tree/main)

The release contains additional installation materials and packaged modules; check release assets and associated documentation before deploying. This branch is a source-code starting point, **not a ready-to-configure hosted service**. Adapt server URLs, secrets, certificates and access controls to your own installation. Never commit credentials to your fork.

## License and contributions

Before distributing derivative builds or source, check the licenses and third-party notices present in this repository and its dependencies. Retain required notices and attribution.

For your own modifications, work in your fork and publish your changes there. To propose improvements to this BloodMoon fork, open a pull request against the appropriate branch and explain compatibility and testing.

**Note:** The presence of a baseline branch is not a guarantee that every feature is supported in every OpenSimulator version or viewer.
