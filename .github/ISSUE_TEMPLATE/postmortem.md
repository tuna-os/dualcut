name: Postmortem Report
description: Document a postmortem after an operational incident resolution
title: '[POSTMORTEM]: '
labels: ['postmortem', 'operations']
body:
  - type: markdown
    attributes:
      value: Postmortem document detailing root cause and action items.
  - type: textarea
    id: root_cause
    attributes:
      label: Root Cause Analysis
      description: Detailed technical root cause.
    validations:
      required: true
  - type: textarea
    id: action_items
    attributes:
      label: Action Items
      description: Preventive measures and improvements.
    validations:
      required: true
