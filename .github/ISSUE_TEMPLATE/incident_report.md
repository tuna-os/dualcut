name: Operational Incident Report
description: Report an operational incident, crash, or engine failure in dualcut
title: '[INCIDENT]: '
labels: ['incident', 'operations']
body:
  - type: markdown
    attributes:
      value: Use this template to report an operational incident or crash.
  - type: textarea
    id: summary
    attributes:
      label: Incident Summary
      description: Brief description of what failed.
    validations:
      required: true
  - type: textarea
    id: impact
    attributes:
      label: User / System Impact
      description: Describe the scope of impact on rendering or video editing.
    validations:
      required: true
  - type: textarea
    id: logs
    attributes:
      label: Diagnostic Logs
      description: Attach relevant stdout/stderr, RUST_LOG, or GST_DEBUG output.
    validations:
      required: false
