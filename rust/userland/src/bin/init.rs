#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    vanta_userland::write(1, b"vanta init: starting /bin/vsh\n");
    let acceptance_ok = native_acceptance();
    let procd = vanta_userland::spawn(b"/bin/procd");
    let procd_ok = procd != u64::MAX && vanta_userland::wait(procd) == 0;
    vanta_userland::write(
        2,
        if procd_ok {
            b"[native] acceptance: procd-gate ok\n"
        } else {
            b"[native] acceptance: procd-gate failed\n"
        },
    );
    let audit_ok = audit_persistence();
    vanta_userland::write(
        2,
        if audit_ok {
            b"[native] acceptance: audit-persistence ok\n"
        } else {
            b"[native] acceptance: audit-persistence failed\n"
        },
    );
    let linux_ok = linux_acceptance();
    vanta_userland::write(
        2,
        if linux_ok {
            b"[linux] Gate C personality acceptance passed\n"
        } else {
            b"[linux] Gate C personality acceptance failed\n"
        },
    );
    let gate_d_ok = gate_d_acceptance();
    vanta_userland::write(
        2,
        if gate_d_ok {
            b"[linux] Gate D dynamic & networking acceptance passed\n"
        } else {
            b"[linux] Gate D dynamic & networking acceptance failed\n"
        },
    );
    let gate = vanta_userland::spawn(b"/bin/native-gate");
    let gate_ok = gate != u64::MAX && vanta_userland::wait(gate) == 0;
    vanta_userland::write(
        2,
        if gate_ok {
            b"[native] acceptance: developer-gate ok\n"
        } else {
            b"[native] acceptance: developer-gate failed\n"
        },
    );
    if acceptance_ok && gate_ok && procd_ok && audit_ok {
        vanta_userland::write(1, b"[native] terminal/filesystem acceptance passed\n");
        vanta_userland::write(1, b"[native] Gate B IPC acceptance passed\n");
    } else {
        vanta_userland::write(2, b"[native] terminal/filesystem acceptance failed\n");
    }
    let shell = vanta_userland::spawn(b"/bin/vsh");
    if shell == u64::MAX {
        vanta_userland::write(2, b"vanta init: could not spawn /bin/vsh\n");
        vanta_userland::exit(1)
    }
    vanta_userland::exit(vanta_userland::wait(shell))
}

fn print_val(label: &[u8], spawn_res: u64, exit_code: u64) {
    vanta_userland::write(2, label);
    vanta_userland::write(2, b": spawn=");
    print_u64(spawn_res);
    vanta_userland::write(2, b" exit=");
    print_u64(exit_code);
    vanta_userland::write(2, b"\n");
}

fn print_u64(mut n: u64) {
    if n == 0 {
        vanta_userland::write(2, b"0");
        return;
    }
    if n == u64::MAX {
        vanta_userland::write(2, b"MAX");
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    let slice = &mut buf[..i];
    slice.reverse();
    vanta_userland::write(2, slice);
}

fn linux_acceptance() -> bool {
    let hello = vanta_userland::spawn_linux(b"/compat/linux/hello");
    let hello_exit = if hello != u64::MAX {
        vanta_userland::wait(hello)
    } else {
        999
    };
    print_val(b"[linux] hello", hello, hello_exit);
    let hello_ok = hello != u64::MAX && hello_exit == 0;

    let cat = vanta_userland::spawn_linux(b"/compat/linux/cat");
    let cat_exit = if cat != u64::MAX {
        vanta_userland::wait(cat)
    } else {
        999
    };
    print_val(b"[linux] cat", cat, cat_exit);
    let cat_ok = cat != u64::MAX && cat_exit == 0;

    let unsupported = vanta_userland::spawn_linux(b"/compat/linux/unsupported");
    let unsupported_exit = if unsupported != u64::MAX {
        vanta_userland::wait(unsupported)
    } else {
        999
    };
    print_val(b"[linux] unsupported", unsupported, unsupported_exit);
    let unsupported_ok = unsupported != u64::MAX && unsupported_exit == 42;

    let ls = vanta_userland::spawn_linux(b"/compat/linux/ls");
    let ls_exit = if ls != u64::MAX {
        vanta_userland::wait(ls)
    } else {
        999
    };
    print_val(b"[linux] ls", ls, ls_exit);
    let ls_ok = ls != u64::MAX && ls_exit == 0;

    let server = vanta_userland::spawn_linux(b"/compat/linux/server");
    let server_exit = if server != u64::MAX {
        vanta_userland::wait(server)
    } else {
        999
    };
    print_val(b"[linux] server", server, server_exit);
    let server_ok = server != u64::MAX && server_exit == 0;

    let musl_hello = vanta_userland::spawn_linux(b"/compat/linux/musl-hello");
    let musl_hello_exit = if musl_hello != u64::MAX {
        vanta_userland::wait(musl_hello)
    } else {
        999
    };
    print_val(b"[linux] musl-hello", musl_hello, musl_hello_exit);
    let musl_hello_ok = musl_hello != u64::MAX && musl_hello_exit == 0;

    let musl_alloc = vanta_userland::spawn_linux(b"/compat/linux/musl-alloc");
    let musl_alloc_exit = if musl_alloc != u64::MAX {
        vanta_userland::wait(musl_alloc)
    } else {
        999
    };
    print_val(b"[linux] musl-alloc", musl_alloc, musl_alloc_exit);
    let musl_alloc_ok = musl_alloc != u64::MAX && musl_alloc_exit == 0;

    let musl_io = vanta_userland::spawn_linux(b"/compat/linux/musl-io");
    let musl_io_exit = if musl_io != u64::MAX {
        vanta_userland::wait(musl_io)
    } else {
        999
    };
    print_val(b"[linux] musl-io", musl_io, musl_io_exit);
    let musl_io_ok = musl_io != u64::MAX && musl_io_exit == 0;

    let musl_dir = vanta_userland::spawn_linux(b"/compat/linux/musl-dir");
    let musl_dir_exit = if musl_dir != u64::MAX {
        vanta_userland::wait(musl_dir)
    } else {
        999
    };
    print_val(b"[linux] musl-dir", musl_dir, musl_dir_exit);
    let musl_dir_ok = musl_dir != u64::MAX && musl_dir_exit == 0;

    let musl_pipe = vanta_userland::spawn_linux(b"/compat/linux/musl-pipe");
    let musl_pipe_exit = if musl_pipe != u64::MAX {
        vanta_userland::wait(musl_pipe)
    } else {
        999
    };
    print_val(b"[linux] musl-pipe", musl_pipe, musl_pipe_exit);
    let musl_pipe_ok = musl_pipe != u64::MAX && musl_pipe_exit == 0;

    let musl_proc = vanta_userland::spawn_linux(b"/compat/linux/musl-proc");
    let musl_proc_exit = if musl_proc != u64::MAX {
        vanta_userland::wait(musl_proc)
    } else {
        999
    };
    print_val(b"[linux] musl-proc", musl_proc, musl_proc_exit);
    let musl_proc_ok = musl_proc != u64::MAX && musl_proc_exit == 0;

    let musl_script = vanta_userland::spawn_linux(b"/compat/linux/musl-script");
    let musl_script_exit = if musl_script != u64::MAX {
        vanta_userland::wait(musl_script)
    } else {
        999
    };
    print_val(b"[linux] musl-script", musl_script, musl_script_exit);
    let musl_script_ok = musl_script != u64::MAX && musl_script_exit == 0;

    let musl_server = vanta_userland::spawn_linux(b"/compat/linux/musl-server");
    let musl_server_exit = if musl_server != u64::MAX {
        vanta_userland::wait(musl_server)
    } else {
        999
    };
    print_val(b"[linux] musl-server", musl_server, musl_server_exit);
    let musl_server_ok = musl_server != u64::MAX && musl_server_exit == 0;
    let dynamic = vanta_userland::spawn_linux(b"/compat/linux/dynamic");
    let dynamic_ok = dynamic == u64::MAX - 1;
    vanta_userland::write(
        2,
        if dynamic_ok {
            b"[linux] dynamic interpreter rejected\n"
        } else {
            b"[linux] dynamic interpreter rejection failed\n"
        },
    );
    hello_ok
        && cat_ok
        && unsupported_ok
        && ls_ok
        && server_ok
        && musl_hello_ok
        && musl_alloc_ok
        && musl_io_ok
        && musl_dir_ok
        && musl_pipe_ok
        && musl_proc_ok
        && musl_script_ok
        && musl_server_ok
        && dynamic_ok
}

fn gate_d_acceptance() -> bool {
    let dyn_hello = vanta_userland::spawn_linux(b"/compat/linux/dynamic-hello");
    let dyn_hello_exit = if dyn_hello != u64::MAX {
        vanta_userland::wait(dyn_hello)
    } else {
        999
    };
    print_val(b"[linux] dynamic-hello", dyn_hello, dyn_hello_exit);
    let dyn_hello_ok = dyn_hello != u64::MAX && dyn_hello_exit == 0;

    let dyn_signal = vanta_userland::spawn_linux(b"/compat/linux/dynamic-signal");
    let dyn_signal_exit = if dyn_signal != u64::MAX {
        vanta_userland::wait(dyn_signal)
    } else {
        999
    };
    print_val(b"[linux] dynamic-signal", dyn_signal, dyn_signal_exit);
    let dyn_signal_ok = dyn_signal != u64::MAX && dyn_signal_exit == 0;

    let dyn_threads = vanta_userland::spawn_linux(b"/compat/linux/dynamic-threads");
    let dyn_threads_exit = if dyn_threads != u64::MAX {
        vanta_userland::wait(dyn_threads)
    } else {
        999
    };
    print_val(b"[linux] dynamic-threads", dyn_threads, dyn_threads_exit);
    let dyn_threads_ok = dyn_threads != u64::MAX && dyn_threads_exit == 0;

    let dyn_net = vanta_userland::spawn_linux(b"/compat/linux/dynamic-net");
    let dyn_net_exit = if dyn_net != u64::MAX {
        vanta_userland::wait(dyn_net)
    } else {
        999
    };
    print_val(b"[linux] dynamic-net", dyn_net, dyn_net_exit);
    let dyn_net_ok = dyn_net != u64::MAX && dyn_net_exit == 0;

    let dyn_fork = vanta_userland::spawn_linux(b"/compat/linux/dynamic-fork");
    let dyn_fork_exit = if dyn_fork != u64::MAX {
        vanta_userland::wait(dyn_fork)
    } else {
        999
    };
    print_val(b"[linux] dynamic-fork", dyn_fork, dyn_fork_exit);
    let dyn_fork_ok = dyn_fork != u64::MAX && dyn_fork_exit == 0;

    let dyn_epoll = vanta_userland::spawn_linux(b"/compat/linux/dynamic-epoll");
    let dyn_epoll_exit = if dyn_epoll != u64::MAX {
        vanta_userland::wait(dyn_epoll)
    } else {
        999
    };
    print_val(b"[linux] dynamic-epoll", dyn_epoll, dyn_epoll_exit);
    let dyn_epoll_ok = dyn_epoll != u64::MAX && dyn_epoll_exit == 0;

    let dyn_proc = vanta_userland::spawn_linux(b"/compat/linux/dynamic-proc");
    let dyn_proc_exit = if dyn_proc != u64::MAX {
        vanta_userland::wait(dyn_proc)
    } else {
        999
    };
    print_val(b"[linux] dynamic-proc", dyn_proc, dyn_proc_exit);
    let dyn_proc_ok = dyn_proc != u64::MAX && dyn_proc_exit == 0;

    let displayd_pid = vanta_userland::spawn(b"/bin/displayd");
    let displayd_exit = if displayd_pid != u64::MAX {
        vanta_userland::wait(displayd_pid)
    } else {
        999
    };
    print_val(b"[desktop] displayd", displayd_pid, displayd_exit);
    let displayd_ok = displayd_pid != u64::MAX && displayd_exit == 0;

    let desktop_pid = vanta_userland::spawn(b"/bin/desktop");
    let desktop_exit = if desktop_pid != u64::MAX {
        vanta_userland::wait(desktop_pid)
    } else {
        999
    };
    print_val(b"[desktop] desktop", desktop_pid, desktop_exit);
    let desktop_ok = desktop_pid != u64::MAX && desktop_exit == 0;

    let audiod_pid = vanta_userland::spawn(b"/bin/audiod");
    let audiod_exit = if audiod_pid != u64::MAX {
        vanta_userland::wait(audiod_pid)
    } else {
        999
    };
    print_val(b"[desktop] audiod", audiod_pid, audiod_exit);
    let audiod_ok = audiod_pid != u64::MAX && audiod_exit == 0;

    dyn_hello_ok
        && dyn_signal_ok
        && dyn_threads_ok
        && dyn_net_ok
        && dyn_fork_ok
        && dyn_epoll_ok
        && dyn_proc_ok
        && displayd_ok
        && desktop_ok
        && audiod_ok
}

fn audit_persistence() -> bool {
    let fd = vanta_userland::open(b"/home/vanta/service-audit.log", vanta_userland::OPEN_READ);
    if fd == u64::MAX - 1 {
        return false;
    }
    let mut bytes = [0_u8; 128];
    let count = vanta_userland::read(fd, &mut bytes);
    vanta_userland::close(fd);
    count > 0
        && contains(&bytes[..count as usize], b"registered\n")
        && contains(&bytes[..count as usize], b"crashed\n")
        && contains(&bytes[..count as usize], b"upgraded\n")
        && contains(&bytes[..count as usize], b"discovered\n")
        && contains(&bytes[..count as usize], b"backend-read\n")
        && contains(&bytes[..count as usize], b"stale-revoked\n")
        && contains(&bytes[..count as usize], b"revoked\n")
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return needle.is_empty();
    }
    let limit = haystack.len() - needle.len();
    let mut offset = 0;
    while offset <= limit {
        if &haystack[offset..offset + needle.len()] == needle {
            return true;
        }
        offset += 1;
    }
    false
}

fn native_acceptance() -> bool {
    let path = b"/home/vanta/init-acceptance";
    let renamed = b"/home/vanta/init-acceptance-renamed";
    let directory = b"/home/vanta/init-acceptance-dir";
    let _ = vanta_userland::mkdir(b"/home/vanta");
    let fd = vanta_userland::open(
        path,
        vanta_userland::OPEN_CREATE | vanta_userland::OPEN_TRUNCATE,
    );
    if fd == u64::MAX - 1 {
        vanta_userland::write(2, b"[native] acceptance: create failed\n");
        return false;
    }
    let _write_result = vanta_userland::write(fd, b"vanta-native\n");
    let mut stat = [0_u8; 16];
    let stat_ok = vanta_userland::fstat(fd, &mut stat) == 0
        && u64::from_ne_bytes(stat[..8].try_into().unwrap()) == 13;
    vanta_userland::close(fd);
    let read_fd = vanta_userland::open(path, vanta_userland::OPEN_READ);
    if read_fd == u64::MAX - 1 {
        vanta_userland::write(2, b"[native] acceptance: read-open failed\n");
        return false;
    }
    let mut bytes = [0_u8; 32];
    let count = vanta_userland::read(read_fd, &mut bytes);
    vanta_userland::close(read_fd);
    let read_ok = count == 13 && &bytes[..13] == b"vanta-native\n";
    let _mkdir_result = vanta_userland::mkdir(directory);
    let directory_fd = vanta_userland::open(directory, vanta_userland::OPEN_READ);
    let dir_ok = directory_fd != u64::MAX - 1;
    if dir_ok {
        vanta_userland::close(directory_fd);
    }
    let rename_ok = vanta_userland::rename(path, renamed) == 0;
    let remove_ok = vanta_userland::unlink(renamed) == 0;
    let remove_dir_ok = vanta_userland::unlink(directory) == 0;
    let c_hello = vanta_userland::spawn(b"/bin/c-hello");
    let c_hello_ok = c_hello != u64::MAX && vanta_userland::wait(c_hello) == 0;
    vanta_userland::write(
        2,
        if c_hello_ok {
            b"[native] acceptance: c-hello ok\n"
        } else {
            b"[native] acceptance: c-hello failed\n"
        },
    );
    let c_sdk_smoke = vanta_userland::spawn(b"/bin/c-sdk-smoke");
    let c_sdk_smoke_ok = c_sdk_smoke != u64::MAX && vanta_userland::wait(c_sdk_smoke) == 0;
    vanta_userland::write(
        2,
        if c_sdk_smoke_ok {
            b"[native] acceptance: c-sdk-smoke ok\n"
        } else {
            b"[native] acceptance: c-sdk-smoke failed\n"
        },
    );
    let c_stdio_smoke = vanta_userland::spawn(b"/bin/c-stdio-smoke");
    let c_stdio_smoke_ok = c_stdio_smoke != u64::MAX && vanta_userland::wait(c_stdio_smoke) == 0;
    vanta_userland::write(
        2,
        if c_stdio_smoke_ok {
            b"[native] acceptance: c-stdio-smoke ok\n"
        } else {
            b"[native] acceptance: c-stdio-smoke failed\n"
        },
    );
    let c_dir_smoke = vanta_userland::spawn(b"/bin/c-dir-smoke");
    let c_dir_smoke_ok = c_dir_smoke != u64::MAX && vanta_userland::wait(c_dir_smoke) == 0;
    vanta_userland::write(
        2,
        if c_dir_smoke_ok {
            b"[native] acceptance: c-dir-smoke ok\n"
        } else {
            b"[native] acceptance: c-dir-smoke failed\n"
        },
    );
    let c_env_smoke = vanta_userland::spawn(b"/bin/c-env-smoke");
    let c_env_smoke_ok = c_env_smoke != u64::MAX && vanta_userland::wait(c_env_smoke) == 0;
    vanta_userland::write(
        2,
        if c_env_smoke_ok {
            b"[native] acceptance: c-env-smoke ok\n"
        } else {
            b"[native] acceptance: c-env-smoke failed\n"
        },
    );
    let c_process_smoke = vanta_userland::spawn(b"/bin/c-process-smoke");
    let c_process_smoke_ok =
        c_process_smoke != u64::MAX && vanta_userland::wait(c_process_smoke) == 0;
    vanta_userland::write(
        2,
        if c_process_smoke_ok {
            b"[native] acceptance: c-process-smoke ok\n"
        } else {
            b"[native] acceptance: c-process-smoke failed\n"
        },
    );
    let c_exec_smoke = vanta_userland::spawn(b"/bin/c-exec-smoke");
    let c_exec_smoke_ok = c_exec_smoke != u64::MAX && vanta_userland::wait(c_exec_smoke) == 0;
    vanta_userland::write(
        2,
        if c_exec_smoke_ok {
            b"[native] acceptance: c-exec-smoke ok\n"
        } else {
            b"[native] acceptance: c-exec-smoke failed\n"
        },
    );
    vanta_userland::write(2, b"w1 ");
    vanta_userland::write(2, if stat_ok { b"s1 " } else { b"s0 " });
    vanta_userland::write(2, if read_ok { b"r1 " } else { b"r0 " });
    vanta_userland::write(2, if dir_ok { b"d1 " } else { b"d0 " });
    vanta_userland::write(2, if rename_ok { b"n1 " } else { b"n0 " });
    vanta_userland::write(2, if remove_ok { b"x1 " } else { b"x0 " });
    vanta_userland::write(2, if remove_dir_ok { b"D1\n" } else { b"D0\n" });
    vanta_userland::write(2, if c_hello_ok { b"c1\n" } else { b"c0\n" });
    stat_ok
        && read_ok
        && dir_ok
        && rename_ok
        && remove_ok
        && remove_dir_ok
        && c_hello_ok
        && c_sdk_smoke_ok
        && c_stdio_smoke_ok
        && c_dir_smoke_ok
        && c_env_smoke_ok
        && c_process_smoke_ok
        && c_exec_smoke_ok
}
