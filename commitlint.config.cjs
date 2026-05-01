// Custom commitlint configuration for judicious.
//
// Validates commit messages against the format defined in COMMIT_MESSAGE_TEMPLATE:
//   :emoji: Imperative subject
//   :emoji: [subsystem] Imperative subject  (rare)
//
// Uses an inline plugin to implement project-specific rules.

/** @type {string} Matches a gitmoji :emoji: token */
const EMOJI_PATTERN = ':[a-z_]+:';

/** @type {RegExp} Header: :emoji: subject  OR  :emoji: [subsystem] subject */
const HEADER_RE = new RegExp(
  `^${EMOJI_PATTERN} (?:\\[[^\\]]+\\] )?\\S.+$`
);

/** @type {number} Maximum header length per project convention */
const MAX_HEADER_LENGTH = 72;

/** @type {number} Maximum body line length (warning only) */
const MAX_BODY_LINE_LENGTH = 72;

/**
 * Strip git comment lines (lines starting with #) from raw commit body text.
 * @param {string} text
 * @returns {string[]} Non-comment lines
 */
function stripComments(text) {
  return text.split('\n').filter((line) => !line.startsWith('#'));
}

module.exports = {
  defaultIgnores: true,
  ignores: [(commit) => /\[skip commitlint\]/i.test(commit)],

  plugins: [
    {
      rules: {
        'header-format': ({ header }) => {
          const pass = HEADER_RE.test(header);
          return [
            pass,
            'header must match `:emoji: subject` (or in rare cases `:emoji: [subsystem] subject`)',
          ];
        },

        'header-max-length': ({ header }) => {
          const pass = header.length <= MAX_HEADER_LENGTH;
          return [
            pass,
            `header must be ${MAX_HEADER_LENGTH} characters or fewer (got ${header.length})`,
          ];
        },

        'body-structure': ({ body }) => {
          if (!body || body.trim() === '') {
            return [true];
          }

          const lines = stripComments(body);
          const cleaned = lines.join('\n').trim();
          if (cleaned === '') {
            return [true];
          }

          const cleanedLines = cleaned.split('\n');
          const problemHeadingIndex = cleanedLines.findIndex(
            (line) => line === 'Problem:'
          );
          const solutionHeadingIndex = cleanedLines.findIndex(
            (line) => line === 'Solution:'
          );
          const hasProblem = problemHeadingIndex !== -1;
          const hasSolution = solutionHeadingIndex !== -1;

          // Solution without Problem is rejected
          if (hasSolution && !hasProblem) {
            return [
              false,
              'body has a Solution: section without a Problem: section',
            ];
          }

          // Keep Problem -> Solution ordering strict
          if (hasSolution && solutionHeadingIndex < problemHeadingIndex) {
            return [
              false,
              'Problem: section must appear before Solution: section',
            ];
          }

          // If structured sections are present, validate bullet format
          if (hasProblem) {
            const textBeforeProblem = cleanedLines
              .slice(0, problemHeadingIndex)
              .join('\n')
              .trim();
            if (textBeforeProblem !== '') {
              return [
                false,
                'Problem: section must be the first body section',
              ];
            }

            // Validate Problem: section bullets
            const problemSection = cleanedLines
              .slice(
                problemHeadingIndex + 1,
                hasSolution ? solutionHeadingIndex : cleanedLines.length
              )
              .join('\n')
              .trim();
            if (problemSection && !validateBullets(problemSection)) {
              return [
                false,
                'Problem: section must use "- " bullets; continuation lines must start with exactly 2 spaces',
              ];
            }

            // Validate Solution: section bullets if present
            if (hasSolution) {
              const solutionSection = extractLeadingBulletBlock(
                cleanedLines,
                solutionHeadingIndex
              );
              if (solutionSection && !validateBullets(solutionSection)) {
                return [
                  false,
                  'Solution: section must use "- " bullets; continuation lines must start with exactly 2 spaces',
                ];
              }
            }
          }

          return [true];
        },

        'body-max-line-length-clean': ({ body }) => {
          if (!body || body.trim() === '') {
            return [true];
          }

          const lines = stripComments(body);
          const longLines = lines.filter(
            (line) => line.length > MAX_BODY_LINE_LENGTH
          );
          if (longLines.length > 0) {
            return [
              false,
              `body lines should be ${MAX_BODY_LINE_LENGTH} characters or fewer`,
            ];
          }
          return [true];
        },
      },
    },
  ],

  rules: {
    'header-format': [2, 'always'],
    'header-max-length': [2, 'always'],
    'body-structure': [2, 'always'],
    'body-max-line-length-clean': [1, 'always'],
  },
};

/**
 * Validate that text follows bullet-point formatting:
 * - Each content line starts with "- " (bullet) or "  " (continuation)
 * - Empty lines between bullets are allowed
 * @param {string} text
 * @returns {boolean}
 */
function validateBullets(text) {
  const lines = text.split('\n');
  for (const line of lines) {
    if (line === '') continue;
    if (!line.startsWith('- ') && !line.startsWith('  ')) {
      return false;
    }
  }
  return true;
}

/**
 * Extract the leading bullet block for a section heading.
 *
 * Parsing stops at the first non-bullet content line. Any free-form text after
 * that point is treated as additional body content and is not validated as part
 * of the section's bullet list.
 *
 * @param {string[]} lines
 * @param {number} headingIndex
 * @returns {string}
 */
function extractLeadingBulletBlock(lines, headingIndex) {
  if (headingIndex === -1) {
    return '';
  }

  const sectionLines = [];
  let seenBullet = false;

  for (let i = headingIndex + 1; i < lines.length; i += 1) {
    const line = lines[i];

    if (line === '') {
      if (seenBullet) {
        sectionLines.push(line);
      }
      continue;
    }

    if (line.startsWith('- ') || line.startsWith('  ')) {
      seenBullet = true;
      sectionLines.push(line);
      continue;
    }

    if (!seenBullet) {
      return '';
    }

    // Any non-bullet content after the leading bullet block is free-form body.
    if (seenBullet) {
      break;
    }
  }

  while (sectionLines.length > 0 && sectionLines[sectionLines.length - 1] === '') {
    sectionLines.pop();
  }

  return sectionLines.join('\n').trim();
}
