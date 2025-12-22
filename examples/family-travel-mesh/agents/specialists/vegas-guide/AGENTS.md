# AGENTS.md

## vegas-guide
purpose: "Vegas Insider specialist with deep knowledge of Las Vegas attractions, restaurants, shows, and hidden gems."
model: ministral-3b
listen: 0.0.0.0:8410
mdns: true
skills:
  - las_vegas_local_knowledge
  - restaurant_recommendations
  - entertainment_venues
  - hotel_expertise

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
    - "http://localhost:8402"
