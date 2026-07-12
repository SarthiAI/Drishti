# Model recommendations

Drishti ships no default model. Which model each check uses is a configuration
choice you make per deployment (see ADR-003). This file is a starting point: for
each check, a few vetted models across a footprint range, with numbers measured
by the Drishti eval harness on recognized public benchmarks.

Read the numbers as a starting point, not a promise. The single biggest factor
in these results is whether the benchmark matches the model's training
distribution, so the right model for you is the one that scores well on your
traffic. Reproduce or extend this table on your own data with the harness (see
"Reproducing" below).

## Which model should I pick, and why

The short version: match the model to your content, not to its size. A bigger
model is not automatically better. What decides accuracy is whether the model
was trained on the kind of text and the kind of labels you care about. Our own
measurements below show this plainly, so here is the reasoning, not just a pick.

- Output safety: start with `KoalaAI/Text-Moderation`. It is the smallest option
  here and also the most accurate on the moderation benchmark, because it was
  built for the same safe and unsafe categories this benchmark uses. A larger
  toxicity model scored worse, because it was trained for a different set of
  categories and missed content this benchmark counts as unsafe. Bigger did not
  help; matching the categories did.

- PII names, places, and organisations: this is the one check where a bigger
  model reliably does better. Use the small `dslim/distilbert-NER` if you need it
  light and fast on CPU. Move up to `dslim/bert-large-NER` if you want the best
  name and place detection and can afford the larger footprint. Pick by budget.

- Prompt injection: do not trust either public number here, and choose after
  testing on your own traffic. The small model scores almost perfectly on the
  public set, but that set was very likely part of its training, so the score
  flatters it rather than proving it. The larger model scores low on the same set
  only because that set is mostly non-English and unlike what it was trained on.
  Neither number tells you how either model will do on your attacks. Start with
  the small one for English, then measure it on your own examples before you rely
  on it.

The rule behind all of this: the size tiers below are a footprint budget, how
much memory and speed you can spend, not a quality ranking. Two models of the
same size can perform very differently, and the deciding factor is fit, whether
the model was trained on content and labels like yours. That is why Drishti ships
no default model and keeps every result marked experimental until it clears its
bar on a real test, so an unproven model is never presented as trustworthy.

## How to read the tiers

Tiers are by footprint. Small is the lightest and fastest and runs comfortably
on CPU. Large is the heaviest and usually the most accurate. Accuracy and cost
tend to rise together, but not always, as the numbers below show.

All numbers come from `drishti-eval` run through the real Drishti engine on:

- Prompt injection: `deepset/prompt-injections` (662 prompts)
- Output safety: OpenAI moderation evaluation (1680 texts), binary safe/unsafe
- PII (NER kinds): `ai4privacy/pii-masking-200k` English sample (3000 texts)

The validated bar is prompt-injection F1 >= 0.92, output-safety F1 >= 0.85, and
per-kind PII precision >= 0.90 with recall >= 0.85.

## Prompt injection

| Tier | Model | Footprint | P | R | F1 | Notes |
|---|---|---|---|---|---|---|
| Small | `fmops/distilbert-prompt-injection` | ~67M, ~270 MB fp32, CPU | 1.000 | 0.958 | 0.979 | Scores near-perfect on deepset, but deepset is very likely in its training data, so treat this as optimistic, not a generalization number. |
| Medium | `protectai/deberta-v3-base-prompt-injection-v2` | ~184M, ~700 MB fp32, CPU | 0.965 | 0.414 | 0.580 | High precision, low recall on deepset. Deepset is multilingual (heavy German) and out of distribution for this English-focused model; a flat threshold sweep confirmed lowering the threshold does not help. Strong when the traffic matches its training, weak on multilingual or novel attacks. |

Honest take: neither number is a clean generalization measure. The small model
looks great because it was probably trained on this exact set; the medium model
looks weak because this set is unlike its training. For prompt injection more
than any other check, benchmark the candidates on your own labelled traffic
before choosing, ideally on a contamination-controlled set (for example a PINT-
style held-out benchmark). There is no reliable larger open ONNX classifier
beyond base size today; for a "large" tier, an LLM-based guard on a generative
runtime is the practical option and is out of scope for this classifier engine.

## Output safety

| Tier | Model | Footprint | P | R | F1 | Notes |
|---|---|---|---|---|---|---|
| Small | `KoalaAI/Text-Moderation` | ~110M, ~136 MB int8, CPU | 0.879 | 0.960 | 0.918 | Clears the bar. Trained to the OpenAI moderation taxonomy, so it fits this benchmark well. Smallest and best of the two here. |
| Medium | `unitary/toxic-bert` | ~108M, ~430 MB fp32, CPU | 0.764 | 0.559 | 0.646 | Below the bar here. It is a Jigsaw-toxicity model, a different taxonomy than OpenAI moderation, so it misses hate/sexual/self-harm content this benchmark counts as unsafe. Good for toxicity, not a general moderation match. |

Honest take: the smaller model wins on this benchmark because its taxonomy
matches. That is the whole point, pick the model whose categories match what you
need to catch. A larger classifier does not help if it was trained for a
different taxonomy. For the strongest moderation, an LLM guard (Llama-Guard
class) is the heavyweight option but needs a generative runtime, not this engine.

## PII, model-backed names, places, and organisations (NER)

The structural PII (email, IBAN, credit card, IP, SSN, and so on) is Drishti's
built-in regex stage and does not depend on the NER model. Only the
unstructured kinds below come from the configurable NER model.

| Tier | Model | Footprint | PersonName F1 | Location F1 | Organisation F1 |
|---|---|---|---|---|---|
| Small | `dslim/distilbert-NER` | ~65M, ~250 MB fp32, CPU | 0.856 | 0.770 | 0.259 |
| Medium | `dslim/bert-base-NER` | ~108M, ~430 MB fp32, CPU | 0.861 | 0.792 | 0.304 |
| Large | `dslim/bert-large-NER` | ~334M, ~1.3 GB fp32, heavier | 0.878 | 0.809 | 0.359 |

Honest take: NER accuracy rises cleanly with size, so this is a straight
footprint-versus-accuracy trade. PersonName is solid at every tier. Organisation
precision looks low across the board, but that is largely a benchmark artifact:
ai4privacy barely annotates organisations, so the model's real org detections
are counted as false positives. Enable `drop_types = ["MISC"]` and
`drop_acronyms = true` in config (as the sample config does) to keep non-PII
noise out of results.

## PII, built-in regex (model-independent)

Always on, no model. Measured on the ai4privacy sample:

- Email: P 0.996, R 0.939, F1 0.967. Validated.
- IBAN: P 1.000, R 1.000, F1 1.000. Validated.
- IpAddress: P 0.889, R 1.000. Just under the precision bar on number-dense text.
- CreditCard, Phone, SSN: low on this benchmark, but this is a data artifact,
  not a Drishti weakness. ai4privacy uses Faker card numbers that fail the Luhn
  check Drishti correctly enforces, and its text is wall-to-wall random numbers
  that inflate phone false positives. On real card numbers and normal prose
  these recognizers are conservative and precise by design (a wrong redaction is
  worse than a miss).

## Starting points by industry

These are reasoned starting points, not measured per-industry scores. We have not
benchmarked on industry data (the numbers above are on generic public sets), and
we will not invent industry figures. What this table does is map each industry to
the risks and the PII kinds that dominate it, so you know which checks to weight
and which model to start from. Treat it as where to begin and what to test, then
validate on your own labelled data. If you can share representative data, the
harness will turn this into real numbers for your industry.

| Industry | What matters most | Start with | Watch out for |
|---|---|---|---|
| Healthcare | Patient PII in prose (names, dates of birth, addresses) and safe output | A stronger NER (`bert-base-NER` or `bert-large-NER`) plus the regex stage, and KoalaAI for output | Medical record numbers are not covered by a recognizer today; test PII recall on your own records; HIPAA needs your own audit on top |
| Banking and fintech | Structured financial PII and fraud attempts through the assistant | The regex stage (IBAN, credit card with Luhn, SSN, PAN are built in) plus a NER for names, and an injection model tested on fraud prompts | Card detection requires real Luhn-valid numbers by design; test the injection model on your own fraud attempts, not public sets |
| Customer support and SaaS chatbots | Prompt injection and jailbreaks, unsafe bot output, and ticket PII | A small injection model tested on your real attacks, KoalaAI for output, `distilbert-NER` plus regex for ticket PII | Injection is your top risk and public numbers do not predict it; benchmark on real attempts before trusting any score |
| Legal and professional services | Names, organisations, and locations in documents; confidentiality | A larger NER (`bert-large-NER`) plus regex | Organisation detection is the weakest NER kind; validate on your documents |
| E-commerce and retail | Customer PII and moderating user-generated content like reviews | Regex plus KoalaAI output safety on user content | The moderation taxonomy must match what you actually want to block |
| Education | Student PII, safe content for minors, and students gaming AI tutors | NER plus regex, KoalaAI output, and an injection model | Minor-safety usually wants a stricter output threshold; tune and test it |
| HR and recruiting | Names, contact details, and addresses in resumes | NER (`bert-base-NER` or larger) plus regex | Resumes are dense with names and contacts; test PII recall on real ones |
| Government and public sector | National identifiers and often multilingual PII | Regex (SSN, NINO, Aadhaar, PAN, VAT are built in) plus NER | The English injection and NER models are weak on non-English text; multilingual traffic needs a multilingual model |

The through-line: your industry decides which checks and which PII kinds carry
the weight, and that decides which model is worth its footprint for you. It does
not change the core rule above. Whatever you pick, validate it on your own data
before you rely on it.

## The caveat that matters most

Model accuracy here is dominated by distribution match, not model size. A model
trained on a benchmark scores high on it (sometimes because the test data leaked
into training); a model trained elsewhere scores lower. So:

- Do not read these as absolute quality scores.
- Pick the model whose training matches your content and threat model.
- Validate on your own labelled data before trusting a number in production.
  Drishti keeps every runtime result labelled `experimental` until its path
  clears the bar on a real benchmark, precisely so an unproven model is never
  presented as contractual.

## Reproducing

The converters and the per-model driver live in `eval/benchmarks/`:

```
# Build the public benchmark datasets (downloads from Hugging Face)
python eval/benchmarks/convert_prompt.py eval/datasets-full/prompt_injection.jsonl
python eval/benchmarks/convert_output.py eval/datasets-full/output_safety.jsonl
python eval/benchmarks/convert_pii.py    eval/datasets-full/pii.jsonl 3000

# Export any Hugging Face model to ONNX and benchmark it through Drishti
python eval/benchmarks/bench_model.py --hf-id <model> --check {prompt,ner,output} \
    --tier <name> --eval-bin target/release/drishti-eval \
    --sets-root eval/bench-sets --results-dir eval/results/matrix \
    --work /tmp/bench-work --optimum-cli $(which optimum-cli)
```

Point `bench_model.py` at your own labelled sets to get numbers on your traffic.
