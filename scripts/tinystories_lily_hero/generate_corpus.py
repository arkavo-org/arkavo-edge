#!/usr/bin/env python3
"""Build a TinyStories-style corpus of grown-up Lily doing everyday heroism."""

from __future__ import annotations

import argparse
import json
import random
from pathlib import Path

OPENINGS = [
    "Once upon a time, Lily grew up and studied.",
    "Once upon a time, Lily was a grown woman who chose college.",
    "Lily was grown now, and her mind was as free as her bicycle.",
    "When Lily grew up, she went to the big college in the city.",
    "Lily had grown into a free-spirited woman who studied science.",
    "Once upon a time, the little girl named Lily became a learned woman.",
]

JOBS = [
    "she studied science for many years and became a doctor at the town clinic",
    "she studied at college and taught science to young people",
    "she became an engineer who had learned how bridges and wheels stand",
    "she studied plants at university and kept a small garden lab",
    "she read every book, then taught at the little college",
    "she studied the stars and still rode her red bicycle to class",
    "she became a doctor who had studied hard and kept wildflowers in her coat",
    "she studied engineering and fixed the town's tools with a song",
]

TRAITS = [
    "She wore bright boots, carried books, and sang while she worked.",
    "She kept wildflowers in her pockets and notes from college in her bag.",
    "She rode her old red bicycle to the university and everywhere else.",
    "No one could tell her she could not learn. She chose her own path.",
    "She let the wind tangle her hair on the way to her studies.",
    "She hummed the songs her mother taught her while she read science.",
]

CHILDHOOD = [
    "She still remembered her brother climbing the tree for her red ball.",
    "She still remembered how her mother held her when she was small and afraid.",
    "When she was a little girl, people had been kind to her, and she never forgot.",
    "She had been nurtured with patience, so she grew patient too.",
    "The love she was given as a child had grown roots in her heart.",
]

NEEDS = [
    ("a lost little boy", "He could not find his mother."),
    ("an old woman named Mrs. Green", "Her bag was too heavy, and her hands were shaking."),
    ("a wet gray cat", "It was stuck under a fence in the rain."),
    ("a shy girl named Sam", "Other children had laughed at her drawing."),
    ("a tired man on the road", "His bicycle wheel was bent."),
    ("a neighbor named Tom", "The wind had blown the roof off his hen house."),
    ("a new family in town", "They looked lost and lonely."),
    ("a little bird", "It had fallen from a nest."),
    ("an old man named Ben", "He could not see well in the dark street."),
    ("a hungry child", "The child had no lunch."),
    ("a woman who dropped her papers", "The wind scattered them down the street."),
    ("two people arguing in the market", "Their words were getting sharp and unkind."),
]

ACTS = [
    "Lily stopped at once. She used what she had studied.",
    "Lily's free heart pulled her close. She was not too busy to care.",
    "Lily knelt down so she could listen, then used her science.",
    "Lily rolled up her sleeves. Her studies had taught her how to help.",
    "Lily spoke in a warm, steady voice. Knowledge made her calm, not proud.",
]

CLOSINGS = [
    "Lily smiled. Learning and kindness were her kind of brave.",
    "She rode home from college in the evening light, tired and glad.",
    "The town felt safer because an educated woman was in it.",
    "Lily knew she was not a knight. She was a neighbor who had studied.",
    "That night she told her mother, and her mother said, 'You grew up just right.'",
    "Lily felt the same sun she had loved as a little girl, and she was proud of her studies.",
]


def _story(rng: random.Random) -> str:
    opening = rng.choice(OPENINGS)
    job = rng.choice(JOBS)
    trait = rng.choice(TRAITS)
    childhood = rng.choice(CHILDHOOD)
    person, trouble = rng.choice(NEEDS)
    act = rng.choice(ACTS)
    close = rng.choice(CLOSINGS)
    place = rng.choice(
        [
            "by the river",
            "on the hill path",
            "near the market",
            "outside the school",
            "in the rain",
            "under the old oak",
            "on the ferry dock",
            "in the quiet library",
        ]
    )
    help_line = rng.choice(
        [
            f"Lily helped {person}. {trouble}",
            f"Then Lily saw {person}. {trouble}",
            f"{place.capitalize()}, Lily met {person}. {trouble}",
        ]
    )
    deed = rng.choice(
        [
            "She used her studies until the problem was smaller.",
            "She stayed until the person felt safe, then taught a young girl she could study too.",
            "She used her hands, her books, her time, and her kind words.",
            "She showed a child how to be brave, and how to learn.",
            "She shared bread, time, and a simple science that helped.",
            "She stood between unkindness and someone smaller, an empowered lady with a trained mind.",
        ]
    )
    thanks = rng.choice(
        [
            "Someone said, 'You are a hero.' Lily shook her head. 'I studied so I could help.'",
            "A child asked, 'Are you magic?' Lily laughed. 'No. I went to college, and I did not walk past.'",
            "People waved as she left. Lily waved back with books under her arm.",
            "No one wrote her name on a statue. That was fine with Lily. Her studies were enough.",
        ]
    )
    return (
        f"{opening} {job[0].upper() + job[1:]}. {trait} {childhood} "
        f"{help_line} {act} {deed} {thanks} {close}"
    )


def generate(n: int, seed: int = 7) -> list[str]:
    rng = random.Random(seed)
    seen: set[str] = set()
    stories: list[str] = []
    # A few fully written anchors so the model hears the target voice, not only slots.
    anchors = [
        (
            "Once upon a time, Lily grew up and studied. She was a free-spirited woman who "
            "rode a red bicycle to college and planted sunflowers behind her shed. When she "
            "was a little girl, her brother had climbed a tree to get her ball, and her "
            "mother had held her when she cried. That love had grown with her, and so had "
            "her mind. She studied science for many years. One windy day she saw Mrs. Green "
            "feel faint in the street. Lily used what she had learned as a doctor, sat with "
            "her, and walked her home. Mrs. Green said Lily was a hero. Lily said, 'I "
            "studied so I could help.' The end."
        ),
        (
            "Lily was a grown woman now. She still loved the sun and still sang. She had "
            "studied engineering at college. A shy boy named Tom came in with a bent wheel "
            "and a red face. Bigger children had laughed at him. Lily knelt down. She used "
            "her studies to fix the wheel and taught Tom how things work. 'You can learn "
            "hard things,' she said. Tom smiled. Lily felt brave in a quiet, educated way. "
            "The end."
        ),
        (
            "When Lily grew up, she went to university. She wore bright boots, kept "
            "wildflowers in her pockets, and read science on the park bench. One evening "
            "she found a lost little girl under the oak. Lily sat with her, taught her the "
            "names of the stars she had studied, and walked her back to her mother. The "
            "mother cried with thanks. Lily remembered being small and found, and she was "
            "glad she had grown kind and learned. The end."
        ),
        (
            "Once upon a time, the little girl named Lily became a learned woman. No one "
            "could tell her she could not go to college. She chose her own path. In the "
            "market two people began to shout. Lily stepped in with a calm voice trained by "
            "years of study. She asked them to take a breath. They did. Everyday heroism, "
            "Lily thought, is a trained mind that does not look away. The end."
        ),
        (
            "Lily had grown into a free-spirited woman who studied to be a doctor. A hungry "
            "child stood by the clinic window. Lily wrapped a warm loaf, a kind word, and a "
            "simple lesson: you can learn, and you can be well. She did not make the child "
            "feel small. She made the child feel seen. That night her mother said, 'You grew "
            "up just right.' The end."
        ),
    ]
    stories.extend(anchors)
    seen.update(anchors)
    while len(stories) < n:
        s = _story(rng)
        if not s.endswith("The end."):
            s = s.rstrip() + " The end."
        if s not in seen:
            seen.add(s)
            stories.append(s)
        if len(seen) > n * 4:
            break
    rng.shuffle(stories)
    return stories


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--n", type=int, default=800)
    parser.add_argument("--seed", type=int, default=7)
    parser.add_argument("--val-frac", type=float, default=0.1)
    args = parser.parse_args()
    stories = generate(args.n, args.seed)
    n_val = max(1, int(len(stories) * args.val_frac))
    val = stories[:n_val]
    train = stories[n_val:]
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "train.json").write_text(json.dumps(train, indent=2) + "\n")
    (args.out / "val.json").write_text(json.dumps(val, indent=2) + "\n")
    print(f"wrote {len(train)} train and {len(val)} val stories to {args.out}")


if __name__ == "__main__":
    main()
