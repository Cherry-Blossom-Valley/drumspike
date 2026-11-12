# Drum Spike 0.9.34

Changes since 0.9.33:

### Export

- The Export button works again after you cancel the "No tempo set" notice. Before, the button could stay disabled (#478).

### Settings

- A detector plug-in row reads as one row: the name on one line and the Licence button at its own width (#479).

### Help and guide

- The Help in the app and the guide match the app: chapters 2 to 14, the agent API page, README and the best-results page say what 0.9.33 does (placement rule version 10, the MIDI export, the agent's second opinion) (#462, #463, #466).
- All pictures of the guide are retaken, with new pictures for the agent fine-tune chapter and the Training Mode chapter (#468).

### Videos and landing page

- Three new explainers: the review queue of doubtful hits (#485), Learn (#484) and tempo, bars and beats (#486).
- The explainers of the first sample, bleed, the agent and Training Mode state the facts of 0.9.33 (#464, #465). No text lies on a drawn line any more in four explainers (#497).
- The agent tutorial is recorded again from one real agent run at 0.9.33: the hit placed by hand that the agent leaves alone, and the hit where its two opinions differ (#482, #483).
- The landing page text for placement, privacy and the agent matches the app (#467).

### Development

- A build of an explainer fails when a text lies on a drawn line; a test checks the wording of all video scripts; the product video refuses a clip from an old explainer build (#481, #494, #497).
