mod process;
mod scan;
mod ui;

use process::Process;
use std::fmt;
use winapi::um::winnt;

/// Environment variable with the process identifier of the process to work with.
/// If the variable if not set (`set PID=...`), it's asked at runtime.
static PROGRAM_PID: Option<&str> = option_env!("PID");

struct ProcessItem {
    pid: u32,
    name: String,
}

impl fmt::Display for ProcessItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (pid={})", self.name, self.pid)
    }
}

fn main() {
    let pid = PROGRAM_PID
        .map(|pid| pid.parse::<u32>().unwrap())
        .unwrap_or_else(|| {
            let processes = process::enum_proc()
                .unwrap()
                .into_iter()
                .flat_map(Process::open)
                .flat_map(|proc| match proc.name() {
                    Ok(name) => Ok(ProcessItem {
                        pid: proc.pid(),
                        name,
                    }),
                    Err(err) => Err(err),
                })
                .collect::<Vec<_>>();

            let item = ui::list_picker(&processes);
            item.pid
        });

    let process = Process::open(pid).unwrap();
    println!("Opened process {:?}", process);

    let mask = winnt::PAGE_EXECUTE_READWRITE
        | winnt::PAGE_EXECUTE_WRITECOPY
        | winnt::PAGE_READWRITE
        | winnt::PAGE_WRITECOPY;

    let regions = process
        .memory_regions()
        .into_iter()
        .filter(|p| (p.Protect & mask) != 0)
        .collect::<Vec<_>>();

    println!("Scanning {} memory regions", regions.len());
    let scan = ui::prompt_scan().unwrap();
    let mut last_scan = process.scan_regions(&regions, scan);
    println!(
        "Found {} locations",
        last_scan.iter().map(|r| r.locations.len()).sum::<usize>()
    );

    while last_scan.iter().map(|r| r.locations.len()).sum::<usize>() > 10 {
        let scan = match ui::prompt_scan() {
            Ok(scan) => scan,
            Err(_) => break,
        };
        last_scan = process.rescan_regions(&last_scan, scan);
        println!(
            "Now have {} locations",
            last_scan.iter().map(|r| r.locations.len()).sum::<usize>()
        );
    }

    // Handle case when we have 2-10 addresses
    let total_locations = last_scan.iter().map(|r| r.locations.len()).sum::<usize>();
    if total_locations > 1 && total_locations <= 10 {
        loop {
            println!("\nFound {} locations. What would you like to do?", total_locations);
            println!("1. Print current values at all locations");
            println!("2. Continue scanning to narrow down further");
            println!("3. Write to all locations");
            
            let choice = ui::prompt::<String>("Enter choice (1/2/3): ");
            match choice.as_ref().map(|s| s.trim()) {
                Ok("1") => {
                    println!("\nCurrent values at found locations:");
                    for (region_idx, region) in last_scan.iter().enumerate() {
                        for (addr_idx, addr) in region.locations.iter().enumerate() {
                            match process.read_memory(addr, 4) {
                                Ok(bytes) => {
                                    let value = i32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                                    println!("  Location {}.{}: [{:x}] = {}", region_idx + 1, addr_idx + 1, addr, value);
                                }
                                Err(e) => {
                                    println!("  Location {}.{}: [{:x}] = Error reading: {}", region_idx + 1, addr_idx + 1, addr, e);
                                }
                            }
                        }
                    }
                }
                Ok("2") => {
                    let scan = match ui::prompt_scan() {
                        Ok(scan) => scan,
                        Err(_) => break,
                    };
                    last_scan = process.rescan_regions(&last_scan, scan);
                    let new_total = last_scan.iter().map(|r| r.locations.len()).sum::<usize>();
                    println!("Now have {} locations", new_total);
                    
                    if new_total == 1 {
                        break;
                    } else if new_total > 10 {
                        // Go back to the main scanning loop
                        while last_scan.iter().map(|r| r.locations.len()).sum::<usize>() > 10 {
                            let scan = match ui::prompt_scan() {
                                Ok(scan) => scan,
                                Err(_) => break,
                            };
                            last_scan = process.rescan_regions(&last_scan, scan);
                            println!(
                                "Now have {} locations",
                                last_scan.iter().map(|r| r.locations.len()).sum::<usize>()
                            );
                        }
                        let total_locations = last_scan.iter().map(|r| r.locations.len()).sum::<usize>();
                        if total_locations > 1 && total_locations <= 10 {
                            continue; // Stay in this menu
                        } else {
                            break; // Exit to final writing phase
                        }
                    }
                    // Continue in this loop if still 2-10 locations
                }
                Ok("3") => break, // Exit to write to all locations
                _ => println!("Invalid choice. Please enter 1, 2, or 3."),
            }
        }
    }

    let new_value = ui::prompt::<i32>("Enter new memory value: ").unwrap();
    let new_value = new_value.to_ne_bytes();
    last_scan.into_iter().for_each(|region| {
        region.locations.iter().for_each(|addr| {
            match process.write_memory(addr, &new_value) {
                Ok(n) => eprintln!("Written {} bytes to [{:x}]", n, addr),
                Err(e) => eprintln!("Failed to write to [{:x}]: {}", addr, e),
            };
        })
    });
}
