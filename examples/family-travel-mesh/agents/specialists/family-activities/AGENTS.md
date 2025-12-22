# AGENTS.md

## family-activities
purpose: "Family activities expert specializing in child-friendly venues, age-appropriate entertainment, and safety assessment."
model: ministral-3b
listen: 0.0.0.0:8411
mdns: true
skills:
  - child_friendly_venues
  - age_appropriate_activities
  - family_restaurants
  - safety_assessment
  - accessibility_knowledge

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
    - "http://localhost:8402"
