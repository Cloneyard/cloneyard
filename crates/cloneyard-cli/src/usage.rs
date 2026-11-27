use std::fmt;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cloneyard_api::usage::{ToolUsage, Tools, ToolsParams};

use crate::connection;
use crate::output::{table, tokens};

/// How far back from now to count, such as `30m`, `12h` or `7d`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Period(Duration);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InvalidPeriod(String);

impl fmt::Display for InvalidPeriod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid period {:?}: a period is a number and m, h or d, such as 7d",
            self.0
        )
    }
}

impl std::error::Error for InvalidPeriod {}

impl FromStr for Period {
    type Err = InvalidPeriod;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidPeriod(text.to_owned());
        let unit = text.chars().last().ok_or_else(invalid)?;
        let seconds = match unit {
            'm' => 60,
            'h' => 3600,
            'd' => 86_400,
            _ => return Err(invalid()),
        };
        let number = &text[..text.len() - 1];
        if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid());
        }
        let count: u64 = number.parse().map_err(|_| invalid())?;
        count
            .checked_mul(seconds)
            .map(|seconds| Self(Duration::from_secs(seconds)))
            .ok_or_else(invalid)
    }
}

/// Prints the tool calls of the clones in `period` up to now, of `project` alone when given.
pub(crate) fn tools(period: Period, project: Option<String>) -> anyhow::Result<()> {
    let params = ToolsParams {
        since: start_of(period, SystemTime::now()),
        project,
    };
    let tools =
        connection::with_client(async |client| Ok(client.request::<Tools>(params).await?))?.tools;
    for line in lines(&tools) {
        println!("{line}");
    }
    Ok(())
}

/// The start of `period` before `now`, in milliseconds since the Unix epoch.
fn start_of(Period(period): Period, now: SystemTime) -> i64 {
    now.checked_sub(period)
        .and_then(|start| start.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

/// A header and a line for each tool, or nothing when there are no tools.
fn lines(tools: &[ToolUsage]) -> Vec<String> {
    if tools.is_empty() {
        return Vec::new();
    }
    let header = [
        "tool",
        "calls",
        "avg result",
        "errors",
        "retries",
        "denials",
    ];
    let mut rows = vec![header.map(str::to_owned).to_vec()];
    rows.extend(tools.iter().map(|tool| {
        let label = match (&tool.command, &tool.path) {
            (Some(command), _) => format!("{}({command} …)", tool.tool),
            (None, Some(path)) => format!("{}({path})", tool.tool),
            (None, None) => tool.tool.clone(),
        };
        let average = tool.average_result_tokens.map_or_else(
            || "-".to_owned(),
            |average| format!("{} tok", tokens(average)),
        );
        vec![
            label,
            tool.calls.to_string(),
            average,
            tool.errors.to_string(),
            tool.retries.to_string(),
            tool.denials.to_string(),
        ]
    }));
    table(&rows, 1..6)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use cloneyard_api::usage::ToolUsage;

    use super::{Period, lines, start_of};

    #[test]
    fn a_period_is_a_number_of_minutes_hours_or_days() {
        let periods = ["30m", "12h", "7d", "0d"].map(|text| text.parse::<Period>().unwrap());

        assert_eq!(
            periods,
            [1800, 43_200, 604_800, 0].map(|seconds| Period(Duration::from_secs(seconds)))
        );
    }

    #[test]
    fn a_period_refuses_what_is_not_a_number_and_a_unit() {
        for text in [
            "",
            "d",
            "7",
            "7x",
            "7 d",
            "-7d",
            "+7d",
            "7dd",
            "1.5h",
            "7ż",
            "99999999999999999999d",
        ] {
            let error = text.parse::<Period>().unwrap_err();

            assert_eq!(
                error.to_string(),
                format!("invalid period {text:?}: a period is a number and m, h or d, such as 7d")
            );
        }
    }

    #[test]
    fn a_period_starts_that_long_before_now_and_never_before_the_epoch() {
        let now = UNIX_EPOCH + Duration::from_millis(1_764_000_000_123);

        let starts = [Duration::from_hours(24), Duration::from_secs(u64::MAX)]
            .map(|period| start_of(Period(period), now));

        assert_eq!(starts, [1_763_913_600_123, 0]);
    }

    fn usage(
        tool: &str,
        command: Option<&str>,
        path: Option<&str>,
        average: Option<u64>,
    ) -> ToolUsage {
        ToolUsage {
            tool: tool.to_owned(),
            command: command.map(str::to_owned),
            path: path.map(str::to_owned),
            calls: 41,
            average_result_tokens: average,
            errors: 6,
            retries: 5,
            denials: 0,
        }
    }

    #[test]
    fn a_tool_shows_with_its_command_start_or_path_under_a_header_and_no_size_as_a_dash() {
        let tools = [
            usage("Bash", Some("gh api"), None, Some(11_200)),
            usage("Read", None, Some("README.md"), Some(980)),
            usage("TodoWrite", None, None, None),
        ];

        assert_eq!(
            lines(&tools),
            [
                "tool             calls  avg result  errors  retries  denials",
                "Bash(gh api …)      41   11.2k tok       6        5        0",
                "Read(README.md)     41     980 tok       6        5        0",
                "TodoWrite           41           -       6        5        0",
            ]
        );
    }

    #[test]
    fn no_tools_print_no_header() {
        assert_eq!(lines(&[]), Vec::<String>::new());
    }
}
