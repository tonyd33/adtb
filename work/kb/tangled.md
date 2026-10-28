# Tangled

kind: system
home: https://tangled.org/

## Static hosting

- Sites deploy static files from a configured repository branch and deploy directory containing index.html; pushes to that branch redeploy. Index sites use the domain root; sub-path sites use the repository name. Directory URLs resolve their index.html automatically. [V 2026-10-27] https://docs.tangled.org/hosting-websites-on-tangled
- Users with tngl.sh accounts get a sites domain; others claim a tngl.io subdomain in Settings/Sites. Custom domains are currently unsupported. A dedicated publication branch updated only by an explicit publish command can implement manual releases; this last sentence is our deployment design inference. [V 2026-10-27] https://docs.tangled.org/hosting-websites-on-tangled

## SSH push authentication

- Knot 2 refreshes users' published SSH key records from their PDS under an hour lease; the older knot could retain copies after records disappeared. Its migration guide advises checking the PDS record, re-adding the same key in Tangled settings to republish it, and allowing the lease to expire. [V 2026-10-28] https://docs.tangled.org/knot-self-hosting-guide
- Tangled's quick start registers an existing public SSH key under Settings/Keys and configures IdentityFile for SSH. Existing authorized identity selection and key publication are distinct checks. [V 2026-10-28] https://docs.tangled.org/quick-start-guide
