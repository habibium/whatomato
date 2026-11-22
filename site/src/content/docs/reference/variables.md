---
title: Message variables
description: Every variable a message or template can hold, what each becomes, and the rules for fallbacks and for moving a time.
---

A variable is a name in double braces. Whatomato fills it in when a message is queued to go out, and [History](/guides/history/) keeps the text as it went. Variables work in scheduled messages, in messages sent right away, in templates, and in messages an agent sends.

## The variables

| Variable | Becomes | Example | Can move |
| --- | --- | --- | --- |
| `{{first_name}}` | The recipient's first name | `Amina` | No |
| `{{name}}` | The recipient's name | `Amina Rahman` | No |
| `{{day}}` | The weekday | `Friday` | Yes |
| `{{date}}` | The day and month, without the year | `27 November` | Yes |
| `{{time}}` | The time, in 12-hour form | `9:05 PM` | Yes |
| `{{month}}` | The month | `November` | Yes |
| `{{year}}` | The year | `2026` | Yes |
| `{{sunrise}}` | Sunrise that day | `6:08 AM` | Yes |
| `{{sunset}}` | Sunset that day | `5:16 PM` | Yes |

Names are case-sensitive: `{{First_Name}}` isn't a variable. Spaces inside the braces are fine: `{{ name }}`.

### Names

- `{{name}}` is the contact's full name as saved on your phone, looked up when the message is queued. For a number that isn't saved, it's the name kept with the message. For a group, it's the group's name as it was when you picked it.
- `{{first_name}}` is the contact's first name as saved on your phone, or failing that the first word of `{{name}}`. **It is always empty for a group.**

### Dates and times

- They describe the moment the message was **due**, not the moment it left. A message that goes out late, after a retry or after the server was down, still gives its planned time. For a message sent right away, it's the moment you sent it.
- They read in the message's time zone.
- Day and month names are in English and times are in 12-hour form with AM or PM, whatever language you or the recipient use.
- `{{sunrise}}` and `{{sunset}}` use the place of a [sun schedule](/reference/schedules/#sun-schedules) if the message has one, and otherwise the main city of its time zone.

## Fallbacks

A fallback goes after a bar and is used when the variable has no value:

```text
Hi {{first_name|there}},
```

says "Hi Amina," to Amina and "Hi there," to a group.

With no value and no fallback, the variable is replaced by nothing, so `Hi {{first_name}}!` goes out as `Hi !`. An empty fallback, `{{first_name|}}`, does the same on purpose and silences the form's warning.

## Moving a time

A time variable can be moved forwards or backwards:

```text
{{sunset + 10m}}
{{time - 30m}}
{{date + 1d}}
{{sunset - 1 hour + 5 min}}
```

The grammar is the variable's name, then any number of moves, then an optional fallback. A move is `+` or `-`, a whole number, and a unit. Spaces are optional.

| Unit | Written as |
| --- | --- |
| Minutes | `m`, `min`, `mins`, `minute`, `minutes` |
| Hours | `h`, `hr`, `hrs`, `hour`, `hours` |
| Days | `d`, `day`, `days` |

Rules:

- Only the time variables move. A name can't.
- A move is at most a year, 366 days in all.
- There are no weeks, months or years as units. Write `+ 7d`.
- Use a plain hyphen for minus. The typographic minus sign that some keyboards and the form's own help text produce isn't accepted.
- Days move on the calendar and minutes on the clock, so `+ 1d` keeps the same clock time across a daylight-saving change.
- `{{sunrise + 1d}}` is the next day's sunrise, not today's sunrise plus 24 hours.

For a message due at 6:00 PM on Sunday 8 November 2026 in Dhaka, where the sun sets at 5:16 PM:

| Written | Goes out as |
| --- | --- |
| `{{sunset + 10m}}` | 5:26 PM |
| `{{time - 30m}}` | 5:30 PM |
| `{{time + 2h}}` | 8:00 PM |
| `{{day + 1d}} {{date + 1 day}}` | Monday 9 November |
| `{{date - 9 days}}` | 30 October |

## What saving refuses

Saving a message or a template checks its variables, and the first problem is reported:

| You wrote | The form says |
| --- | --- |
| `{{firstname}}` | {{firstname}} isn't a variable |
| `{{}}` | {{}} needs a variable's name inside |
| `Hi {{name` | A {{ needs a }} to close it |
| `{{sunset + soon}}` | {{sunset + soon}} should move by a number and a unit, like + 10m |
| `{{first_name + 1d}}` | {{first_name}} isn't a time, so it can't move |
| `{{date + 367d}}` | {{date + 367d}} moves a time by more than a year |

A move needs its sign: `{{sunset 10m}}` is read as an unknown name.

## In the form

Typing `{{` opens the list of variables, which narrows as you type. Each `{{…}}` is tinted blue when it's valid and red when it isn't. Once the text holds a valid variable, the form shows the message filled in under **Goes out as**, and warns you about a variable that would come out empty for the recipient you chose.
