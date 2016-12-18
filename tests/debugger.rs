extern crate gluon_language_server;
extern crate debugserver_types;
extern crate languageserver_types;

extern crate jsonrpc_core;
extern crate serde_json;
extern crate serde;
extern crate url;

#[macro_use]
extern crate lazy_static;

#[allow(dead_code)]
mod support;

use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::io::{BufRead, BufReader, Write};
use std::sync::Mutex;

use serde_json::{Value, from_str};

use debugserver_types::*;

use gluon_language_server::rpc::read_message;

macro_rules! request {
    ($stream: expr, $id: ident, $command: expr, $seq: expr, $expr: expr) => {
        let request = $id {
            arguments: $expr,
            command: $command.to_string(),
            seq: { $seq += 1; $seq },
            type_: "request".into(),
        };
        support::write_message($stream, request).unwrap();
    }
}

macro_rules! expect_response {
    ($read: expr, $typ: ty, $name: expr) => { {
        let msg: $typ = expect_message(&mut $read);
        assert_eq!(msg.command, $name);
        msg
    } }
}

macro_rules! expect_event {
    ($read: expr, $typ: ty, $event: expr) => { {
        let event: $typ = expect_message(&mut $read);
        assert_eq!(event.event, $event);
        event
    } }
}

lazy_static! {
    static ref PORT: Mutex<i32> = Mutex::new(4711);
}

fn run_debugger<F>(f: F)
    where F: FnOnce(&mut i64, &TcpStream, &mut BufReader<&TcpStream>),
{
    let port = {
        let mut port = PORT.lock().unwrap();
        *port += 1;
        *port
    };
    let path = PathBuf::from(::std::env::args().next().unwrap());
    let debugger = path.parent()
        .and_then(|path| path.parent())
        .expect("debugger executable")
        .join("debugger");

    let mut child = Command::new(&debugger)
        .arg(port.to_string())
        .spawn()
        .unwrap_or_else(|_| panic!("Expected exe: {}", debugger.display()));

    let stream = TcpStream::connect(&format!("localhost:{}", port)[..]).unwrap();

    let mut seq = 0;
    let mut read = BufReader::new(&stream);

    request! {
        &stream,
        InitializeRequest,
        "initialize",
        seq,
        InitializeRequestArguments {
            adapter_id: "".into(),
            columns_start_at_1: None,
            lines_start_at_1: None,
            path_format: None,
            supports_run_in_terminal_request: None,
            supports_variable_paging: None,
            supports_variable_type: None,
        }
    };

    let _: InitializedEvent = expect_message(&mut read);

    let initialize_response: InitializeResponse = expect_message(&mut read);
    assert!(initialize_response.success);

    f(&mut seq, &stream, &mut read);

    request! {
        &stream,
        DisconnectRequest,
        "disconnect",
        seq,
        None
    };
    expect_response!(read, DisconnectResponse, "disconnect");

    child.wait().unwrap();
}

fn launch<W>(stream: W, seq: &mut i64, program: &str)
    where W: Write,
{
    request! {
        stream,
        Request,
        "launch",
        *seq,
        Some(Value::Object(vec![("program".to_string(),
                                    Value::String(program.to_string()))]
                                .into_iter()
                                .collect()))
    };
}

fn request_debug_info(seq: &mut i64,
                      stream: &TcpStream,
                      mut read: &mut BufReader<&TcpStream>)
                      -> (StackTraceResponse, ScopesResponse, VariablesResponse) {
    request! {
        stream,
        ThreadsRequest,
        "threads",
        *seq,
        None
    };
    expect_response!(read, ThreadsResponse, "threads");

    request! {
        stream,
        StackTraceRequest,
        "stackTrace",
        *seq,
        StackTraceArguments {
            levels: Some(20),
            start_frame: None,
            thread_id: 1,
        }
    };
    let trace = expect_response!(read, StackTraceResponse, "stackTrace");

    request! {
        stream,
        ScopesRequest,
        "scopes",
        *seq,
        ScopesArguments { frame_id: 0 }
    };
    let scopes = expect_response!(read, ScopesResponse, "scopes");

    request! {
        stream,
        VariablesRequest,
        "variables",
        *seq,
        VariablesArguments {
            count: None,
            filter: None,
            start: None,
            variables_reference: 1
        }
    };
    let variables = expect_response!(read, VariablesResponse, "variables");
    (trace, scopes, variables)
}

fn expect_message<M, R>(read: R) -> M
    where M: serde::Deserialize,
          R: BufRead,
{
    let value = read_message(read).unwrap().unwrap();
    from_str(&value).unwrap_or_else(|err| {
        panic!("{} in message:\n{}", err, value);
    })
}

#[test]
fn launch_program() {
    run_debugger(|seq, stream, mut read| {
        launch(stream, seq, "tests/main.glu");

        let launch_response: LaunchResponse = expect_message(&mut read);
        assert_eq!(launch_response.request_seq, *seq);
        assert!(launch_response.success);

        request! {
            stream,
            ConfigurationDoneRequest,
            "configurationDone",
            *seq,
            None
        };
        let _: ConfigurationDoneResponse = expect_message(&mut read);

        let _: TerminatedEvent = expect_message(&mut read);
    });
}

#[test]
fn infinite_loops_are_terminated() {
    run_debugger(|seq, stream, mut read| {
        launch(stream, seq, "tests/infinite_loop.glu");

        let launch_response: LaunchResponse = expect_message(&mut read);
        assert_eq!(launch_response.request_seq, *seq);
        assert!(launch_response.success);

        request! {
            stream,
            ConfigurationDoneRequest,
            "configurationDone",
            *seq,
            None
        };
        let _: ConfigurationDoneResponse = expect_message(&mut read);
    });
}

#[test]
fn pause() {
    run_debugger(|seq, stream, mut read| {
        launch(stream, seq, "tests/infinite_loop.glu");

        let launch_response: LaunchResponse = expect_message(&mut read);
        assert_eq!(launch_response.request_seq, *seq);
        assert!(launch_response.success);

        request! {
            stream,
            ConfigurationDoneRequest,
            "configurationDone",
            *seq,
            None
        };
        let _: ConfigurationDoneResponse = expect_message(&mut read);

        request! {
            stream,
            PauseRequest,
            "pause",
            *seq,
            PauseArguments { thread_id: 0, }
        };
        let _: PauseResponse = expect_message(&mut read);

        let _: StoppedEvent = expect_message(&mut read);

        request! {
            stream,
            ContinueRequest,
            "continue",
            *seq,
            ContinueArguments { thread_id: 0, }
        };
        let _: ContinueResponse = expect_message(&mut read);
    });
}

#[test]
fn breakpoints() {
    run_debugger(|seq, stream, mut read| {
        launch(stream, seq, "tests/main.glu");

        let launch_response = expect_response!(read, LaunchResponse, "launch");
        assert_eq!(launch_response.request_seq, *seq);
        assert!(launch_response.success);

        request! {
            stream,
            SetBreakpointsRequest,
            "setBreakpoints",
            *seq,
            SetBreakpointsArguments {
                breakpoints: Some(vec![
                    SourceBreakpoint {
                        column: None,
                        condition: None,
                        hit_condition: None,
                        line: 1,
                    },
                    SourceBreakpoint {
                        column: None,
                        condition: None,
                        hit_condition: None,
                        line: 14,
                    },
                ]),
                lines: None,
                source: Source {
                    path: Some("tests/main.glu".into()),
                    .. Source::default()
                },
                source_modified: None,
            }
        };
        expect_response!(read, SetBreakpointsResponse, "setBreakpoints");

        request! {
            stream,
            ConfigurationDoneRequest,
            "configurationDone",
            *seq,
            None
        };
        expect_response!(read, ConfigurationDoneResponse, "configurationDone");

        let stopped = expect_event!(read, StoppedEvent, "stopped");
        assert_eq!(stopped.body.reason, "breakpoint");

        request! {
            stream,
            ContinueRequest,
            "continue",
            *seq,
            ContinueArguments { thread_id: 0, }
        };
        expect_response!(read, ContinueResponse, "continue");

        let stopped = expect_event!(read, StoppedEvent, "stopped");
        assert_eq!(stopped.body.reason, "breakpoint");

        request! {
            stream,
            ContinueRequest,
            "continue",
            *seq,
            ContinueArguments { thread_id: 0, }
        };
        expect_response!(read, ContinueResponse, "continue");

        expect_event!(read, TerminatedEvent, "terminated");
    });
}

#[test]
fn step_in() {
    run_debugger(|seq, stream, mut read| {
        launch(stream, seq, "tests/main.glu");

        let launch_response = expect_response!(read, LaunchResponse, "launch");
        assert_eq!(launch_response.request_seq, *seq);
        assert!(launch_response.success);

        request! {
            stream,
            SetBreakpointsRequest,
            "setBreakpoints",
            *seq,
            SetBreakpointsArguments {
                breakpoints: Some(vec![
                    SourceBreakpoint {
                        column: None,
                        condition: None,
                        hit_condition: None,
                        line: 14,
                    },
                ]),
                lines: None,
                source: Source {
                    path: Some("tests/main.glu".into()),
                    .. Source::default()
                },
                source_modified: None,
            }
        };
        expect_response!(read, SetBreakpointsResponse, "setBreakpoints");

        request! {
            stream,
            ConfigurationDoneRequest,
            "configurationDone",
            *seq,
            None
        };
        expect_response!(read, ConfigurationDoneResponse, "configurationDone");

        let stopped = expect_event!(read, StoppedEvent, "stopped");
        assert_eq!(stopped.body.reason, "breakpoint");

        request! {
            stream,
            StepInRequest,
            "stepIn",
            *seq,
            StepInArguments {
                target_id: None,
                thread_id: 0
            }
        };
        expect_response!(read, StepInResponse, "stepIn");
        let stopped = expect_event!(read, StoppedEvent, "stopped");
        assert_eq!(stopped.body.reason, "step");

        let (trace, _, _) = request_debug_info(seq, stream, read);
        let frames = &trace.body.stack_frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].line, 6);
        assert_eq!(frames[0].name, "test");
        assert_eq!(frames[1].line, 14);

        request! {
            stream,
            ContinueRequest,
            "continue",
            *seq,
            ContinueArguments { thread_id: 0, }
        };
        expect_response!(read, ContinueResponse, "continue");

        expect_event!(read, TerminatedEvent, "terminated");
    });
}

#[test]
fn step_out() {
    run_debugger(|seq, stream, mut read| {
        launch(stream, seq, "tests/main.glu");

        let launch_response = expect_response!(read, LaunchResponse, "launch");
        assert_eq!(launch_response.request_seq, *seq);
        assert!(launch_response.success);

        request! {
            stream,
            SetBreakpointsRequest,
            "setBreakpoints",
            *seq,
            SetBreakpointsArguments {
                breakpoints: Some(vec![
                    SourceBreakpoint {
                        column: None,
                        condition: None,
                        hit_condition: None,
                        line: 6,
                    },
                ]),
                lines: None,
                source: Source {
                    path: Some("tests/main.glu".into()),
                    .. Source::default()
                },
                source_modified: None,
            }
        };
        expect_response!(read, SetBreakpointsResponse, "setBreakpoints");

        request! {
            stream,
            ConfigurationDoneRequest,
            "configurationDone",
            *seq,
            None
        };
        expect_response!(read, ConfigurationDoneResponse, "configurationDone");

        let stopped = expect_event!(read, StoppedEvent, "stopped");
        assert_eq!(stopped.body.reason, "breakpoint");

        request! {
            stream,
            StepOutRequest,
            "stepOut",
            *seq,
            StepOutArguments {
                thread_id: 0
            }
        };
        expect_response!(read, StepOutResponse, "stepOut");
        let stopped = expect_event!(read, StoppedEvent, "stopped");
        assert_eq!(stopped.body.reason, "step");

        let (trace, _, _) = request_debug_info(seq, stream, read);
        let frames = &trace.body.stack_frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].line, 15);

        request! {
            stream,
            ContinueRequest,
            "continue",
            *seq,
            ContinueArguments { thread_id: 0, }
        };
        expect_response!(read, ContinueResponse, "continue");

        expect_event!(read, TerminatedEvent, "terminated");
    });
}

#[test]
fn step_over() {
    run_debugger(|seq, stream, mut read| {
        launch(stream, seq, "tests/main.glu");

        let launch_response = expect_response!(read, LaunchResponse, "launch");
        assert_eq!(launch_response.request_seq, *seq);
        assert!(launch_response.success);

        request! {
            stream,
            SetBreakpointsRequest,
            "setBreakpoints",
            *seq,
            SetBreakpointsArguments {
                breakpoints: Some(vec![
                    SourceBreakpoint {
                        column: None,
                        condition: None,
                        hit_condition: None,
                        line: 14,
                    },
                ]),
                lines: None,
                source: Source {
                    path: Some("tests/main.glu".into()),
                    .. Source::default()
                },
                source_modified: None,
            }
        };
        expect_response!(read, SetBreakpointsResponse, "setBreakpoints");

        request! {
            stream,
            ConfigurationDoneRequest,
            "configurationDone",
            *seq,
            None
        };
        expect_response!(read, ConfigurationDoneResponse, "configurationDone");

        let stopped = expect_event!(read, StoppedEvent, "stopped");
        assert_eq!(stopped.body.reason, "breakpoint");

        request! {
            stream,
            NextRequest,
            "next",
            *seq,
            NextArguments {
                thread_id: 0
            }
        };
        expect_response!(read, StepOutResponse, "next");
        let stopped = expect_event!(read, StoppedEvent, "stopped");
        assert_eq!(stopped.body.reason, "step");

        let (trace, _, _) = request_debug_info(seq, stream, read);
        let frames = &trace.body.stack_frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].line, 15);

        request! {
            stream,
            ContinueRequest,
            "continue",
            *seq,
            ContinueArguments { thread_id: 0, }
        };
        expect_response!(read, ContinueResponse, "continue");

        expect_event!(read, TerminatedEvent, "terminated");
    });
}
