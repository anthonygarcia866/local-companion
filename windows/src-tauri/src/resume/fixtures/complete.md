# HANDOFF (updated 2026-10-12)

## Goal
Ship the tenant portal's maintenance-request form.

## Branch + last commit
- Branch: `feature/maintenance-form`
- Last commit: 3f9c2ab Add photo upload to the request form

## Done
- [x] Form fields and validation
- [x] Email to the property manager

## In progress
Wiring the photo upload to storage; the signed-URL call works locally.
Next: run the e2e test against staging, then open the PR.

## Open questions
- Max photo size: 5 MB or 10 MB?
- Should tenants see the vendor's name?

## Gotchas
- The storage key is in .dev.vars (never here).
