# 弄玉's emotions and gestures

弄玉's motion is data-driven. At load, the renderer reads what her rig offers: parameter names and groups, expressions and motion groups. It then maps 18 emotions and 12 gestures onto them. Anything it finds is used. Anything it can't find falls back to head, body, eye and breathing motion, which works on every rig. She is never still. Between tasks she shifts her weight, tilts her head, looks around, smiles and fidgets.

`/status` shows what was matched, under `rig` in the Live2D details. The same record is saved to `diagnostics/live2d.json` in Aster's state directory. If a match is wrong, correct it with the [override file](#override-file).

## Emotions

`neutral`, `happy`, `laugh`, `shy`, `love`, `excited`, `surprised`, `confused`, `thinking`, `sad`, `cry`, `angry`, `pout`, `embarrassed`, `sleepy`, `proud`, `worried`, `dizzy`

Each emotion has up to three layers:

1. **A pose.** Head angles and tilt, gaze, eyelids, eye smile, sway and breathing rate, plus a few extras. Excited and laughing she bounces lightly, laughing opens her mouth in bursts, crying adds small sobs, sleepy has her dozing with slow blinks, dizzy circles her head and eyes, and surprise jolts her head up and holds her eyes open.
2. **Rig toggles.** Parameters found by keyword ([below](#keyword-table)) ease to their maximum, or a partial weight, over about 0.25 s, then back to default.
3. **An expression and a motion.** A model expression whose name matches the emotion (for example `Happy`, `害羞`) is applied while the emotion lasts. A matching motion group plays once when the emotion begins.

There are two layers of mood:

- **Base mood**: the `mood` argument of `Companion::motion`. It persists.
- **Transient emotion**: `Companion::emote(name, seconds)`. It overrides the base mood until it expires, then she eases back.

Both accept any emotion above, an alias (`heart` → love, `blush` → shy, `tears` → cry, `surprise`, `think`, `sleep`, `worry`, …), an emotion defined in the override file, or the exact name of a model expression. Unknown names leave her neutral and add a warning to `/status`.

## Gestures

`wave`, `nod`, `shake`, `tilt`, `think` (hand to chin), `cheer`, `heart` (hand heart), `cover` (hands over mouth), `stretch`, `look_around`, `fidget`, `bow`

`Companion::act(name)` queues a gesture. It also accepts the exact name of a model motion group. Gestures play one after another. Each is realized, in order of preference, by:

1. **motion**: a model motion group whose name matches (`wave`/`挥手`/`招手`, `cheer`/`欢呼`, `bow`/`鞠躬`, `think`/`思考`/`托腮`, `heart`/`比心`, …). It is played at force priority.
2. **params**: arm and hand parameters found by keyword, animated over time.
   - `wave` raises an arm and swings the waving parameter (or the raised arm itself).
   - `cheer` raises both arms with a small bounce.
   - `think` uses a chin (`托腮`) parameter.
   - `heart` uses a hand-heart (`比心`) parameter.
   - `cover` uses a covering (`捂`) parameter.
   - `stretch` raises the arms.

   A head and body accent plays along.
3. **procedural**: a head and body version. `nod`, `shake`, `tilt`, `look_around`, `bow`, `stretch` and `fidget` always have one. Without motions or arm parameters:
   - `wave` becomes a friendly head sway and smile.
   - `cheer` becomes a bounce with the excited toggles.
   - `heart` becomes a tilt with the love toggles.
   - `cover` becomes a bashful dip with the shy toggles.
   - `think` becomes a chin-up tilt with an upward gaze.

`rig.gestures` records which realization each gesture received (`kind`).

"Raise" means moving an arm parameter from its default toward its maximum. If her arm parameters raise toward their minimum instead, reverse them with the override file.

## Never standing still

- **Weight and posture.** Weight shifts (body X/Z) every 4–9 s, and head tilts every 5–12 s. Small smiles come every 8–20 s when she is calm.
- **Idle fidgets.** Every 12–30 s while she is idle or waiting, she does a fidget, tilt or look-around, occasionally a stretch, or an idle-group motion from the rig.
- **Speaking.** Her head moves with speech energy, and stressed syllables carry a small nod.
- **Listening.** She leans in, with her head slightly lowered and tilted and her eyes up toward you.
- **Blending.** Artist motions blend additively underneath the procedural layer. Per-frame changes stay small.

## Keyword table

A parameter is matched on its id and its DisplayInfo name. Latin keywords match whole words, a plural, or a longer word they begin (five letters or more). Chinese and Japanese keywords match anywhere in the name.

| Toggle | Keywords | Used by |
| --- | --- | --- |
| blush | blush, cheek, flush, 脸红, 红晕, 腮红, 害羞, 红脸, 羞红, 赤面, 照れ | shy 1, embarrassed 1, love .7, laugh .5, pout .5, excited .4 |
| tear | tear, teer, cry, weep, 泪, 哭, 涙 | cry 1, sad .5 |
| heart | heart, love, 爱心, 心 (not 中心, 重心, 比心, 心情), ハート | love 1 |
| star | star, sparkle, twinkle, glitter, 星, 闪闪, キラ | excited 1, proud .6 |
| angry | angry, anger, mad, rage, 生气, 怒, 黑脸, 青筋 | angry 1 |
| sweat | sweat, 汗 | worried 1, embarrassed .8, confused .5, dizzy .5 |
| surprise | surprise, shock, 惊, びっくり | surprised 1 |
| question | question, doubt, 问号, 疑, ？ | confused 1 |
| dizzy | dizzy, spiral, swirl, 晕, 圈圈, 眩, ぐるぐる | dizzy 1 |
| smile | smile, happy, joy, 笑, 眯, 开心 (never a mouth parameter) | laugh 1, happy .6, excited .6, proud .5 |
| pout | pout, 嘟嘴, 嘟, 撅嘴 | pout 1 |
| tongue | tongue, 吐舌, 舌 | embarrassed .8 |
| sleepy | sleepy, sleep, tired, drowsy, zzz, 困, 睡, 眠 | sleepy 1 |
| shadow | shadow, gloom, 阴影, 阴暗, 黑线 | sad .7, angry .5, cry .4, worried .4 |

Arms and hands: arm, hand, elbow, wrist, 手, 臂, 胳膊, 肘, 腕, 挥, 比心, 托腮.

| Role | Keywords |
| --- | --- |
| chin | chin, 托腮, 腮, 下巴 |
| heart | heart, 比心, 爱心 |
| cover | cover, 捂, 遮 |
| wave | wave, 挥, 摆手, 招手 |
| raise | raise, lift, up, 抬, 举 |

Side comes from 左/右 in the name, or from `L`/`R`/`Left`/`Right` in the id.

Earlier tuning for this rig is kept alongside the keywords:

| Parameter | Emotions |
| --- | --- |
| `ParamHeartEye` | love |
| `ParamTeers3` | love, angry |
| `ParamTeers2` | angry, cry, sad .6 |
| `ParamTeers5` | angry, cry |

**Never driven:** `ParamMouthForm`, any parameter with `Brow` in its id, and any parameter whose name contains 眉 or 嘴型. This rig's mouth-form and brow deformers are asymmetric. Parameters driven by her procedural motion are also left out of keyword matching and overrides: angles, body, eye balls, eyelids, eye smile, breath and mouth opening.

## The `rig` catalog

```json
{
  "parameters": [{"id": "ParamCheek", "name": "脸红", "group": "表情", "min": 0, "max": 1, "default": 0}],
  "expressions": ["Happy", "害羞"],
  "motions": {"Idle": 1, "Wave": 1},
  "protected": ["ParamMouthForm", "ParamBrowLY"],
  "categories": {"blush": ["ParamCheek"], "tear": ["ParamTear", "ParamTeers2"]},
  "arms": [{"id": "ParamArmRA", "side": "R", "role": "raise"}],
  "emotions": {"shy": {"params": [{"id": "ParamCheek", "value": 1}], "expression": "害羞", "motion": null}},
  "gestures": {
    "wave": {"params": [], "motion": "Wave", "kind": "motion"},
    "think": {"params": [{"id": "ParamHandChin", "value": 1}], "motion": null, "kind": "params"}
  }
}
```

Toggle `value`s are absolute parameter values. A gesture parameter has a `value` it holds, or `values` it moves through (see below). Entries changed by the override file carry `"override": true`.

## Override file

Aster reads the first of these that exists:

1. `aster-nongyu.json` in the model asset directory (`--pet-dir`, `ASTER_PET_DIR`, default `~/desktop-pet/assets`)
2. `~/.config/aster/nongyu.json`

The file must be a JSON object of at most 64 KB. It is read when the renderer starts, so use `/pet retry` after editing it. Overrides merge over what was found automatically:

- `params` values are absolute. A number sets or replaces a parameter; `null` removes an automatically found one.
- For gestures a value may also be a list:
  - one number: hold it
  - two numbers: swing between them
  - three to eight numbers: keyframes spread over the gesture
- `expression` (emotions only) and `motion` name a model expression or motion group exactly, ignoring case. `null` turns the automatic match off.
- New names add custom emotions or gestures, usable with `emote`, `motion`'s mood and `act`.

Unknown keys, wrong types, unknown parameters and protected parameters are ignored with a warning. Aster's warnings appear under `emotion_map` in `/status`, the renderer's under `warnings`.

```json
{
  "emotions": {
    "shy":   {"params": {"ParamCheek": 1, "ParamTeers3": null}, "expression": "害羞"},
    "proud": {"params": {"ParamStarEye": 0.5}, "motion": null},
    "smug":  {"params": {"ParamStarEye": 0.3}, "expression": "F05"}
  },
  "gestures": {
    "wave":  {"motion": "TapBody"},
    "heart": {"params": {"ParamArmRA": [3, 7]}},
    "salute": {"params": {"ParamArmRA": [0, 8, 8, 0]}}
  }
}
```

## For code

```rust
live2d::EMOTIONS          // the 18 canonical emotions
live2d::GESTURES          // the 12 canonical gestures
companion.motion(state, mood, tap, look); // mood: any emotion, alias or expression name
companion.emote("surprised", 2.0);        // transient; 0 or less clears it
companion.act("wave");                    // one-shot, queued (at most eight waiting)
```

Every frame, `Shared.info` reports `emotion` (in effect), `gesture` (`{name, kind, source}` or null), `actions` (gestures, fidgets and idle motions started so far) and `last_action`. `info["rig"]` holds the catalog after load.

## What is and isn't verified

An automated check runs the real renderer in headless Chromium, with a stub rig whose Chinese display names cover the categories above. It confirms discovery, each emotion's toggles rising and resetting, gestures by motion, parameters and procedure, protected parameters never moving, idle life with a fidget within 35 s, smooth per-frame changes, and the frame rate. How the matches look on the real rig, including whether "raise" is the right direction for her arms, can only be judged by looking at her.
