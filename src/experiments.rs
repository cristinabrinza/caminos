
use std::fmt;
use std::fs::{self,File,OpenOptions};
use std::str::FromStr;
use std::io::prelude::*;
use std::io::{stdout,BufReader};
use std::path::{Path,PathBuf};
use std::process::Command;
use std::net::TcpStream;
use std::collections::{HashSet};

use ssh2::Session;
use indicatif::{ProgressBar,ProgressStyle};

use crate::config_parser::{self,ConfigurationValue};
use crate::{Simulation,Plugs};
use crate::output::{create_output};
use crate::config::{self,evaluate,flatten_configuration_value};

#[derive(Debug,Clone,Copy,PartialEq)]
pub enum Action
{
	///Default action of executing locally and creating the output files.
	LocalAndOutput,
	///Execute remaining runs locally and sequentially.
	Local,
	///Just generates the output with the available data
	Output,
	///Package the executions into Slurm jobs and send them to the Slurm queue system.
	Slurm,
	///Checks how many results it has.
	///TODO: implement looking at slurm error files.
	Check,
	///Bring results from the remote via sftp.
	Pull,
	///Performs a check action on the remote.
	RemoteCheck,
	///Push data into the remote.
	Push,
	///Cancel all slurm jobs owned by the experiment.
	SlurmCancel,
	///Create shell/skeleton/carcase files. This is, create a folder containing the files: main.cfg, main.od, remote. Use `--source` to copy them from a existing one.
	Shell,
	///Builds up a `binary.results` if it does not exists and erase all `runs/run*/`.
	Pack,
}

impl FromStr for Action
{
	type Err = ();
	fn from_str(s:&str) -> Result<Action,()>
	{
		match s
		{
			"default" => Ok(Action::LocalAndOutput),
			"local_and_output" => Ok(Action::LocalAndOutput),
			"local" => Ok(Action::Local),
			"output" => Ok(Action::Output),
			"slurm" => Ok(Action::Slurm),
			"check" => Ok(Action::Check),
			"pull" => Ok(Action::Pull),
			"remote_check" => Ok(Action::RemoteCheck),
			"push" => Ok(Action::Push),
			"slurm_cancel" => Ok(Action::SlurmCancel),
			"shell" => Ok(Action::Shell),
			"pack" => Ok(Action::Pack),
			_ => Err(()),
		}
	}
}

impl fmt::Display for Action
{
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result
	{
		write!(f, "{:?}", *self)
		//match *self
		//{
		//	Action::LocalAndOutput=>write!(f,"LocalAndOutput"),
		//	Action::Local=>write!(f,"Local"),
		//}
	}
}

struct KeyboardInteration;

impl KeyboardInteration
{
	fn ask_password(&self, username: &str, hostname: &str) -> String
	{
		println!("Asking username {} for their password at {}.",username,hostname);
		stdout().lock().flush().unwrap();
		//let mut line = String::new();
		//stdin().lock().read_line(&mut line).unwrap();
		//line
		//XXX We do not want to have echo or similar.
		//FIXME: ^C behaves weird on rpassword
		rpassword::read_password_from_tty(Some("Password: ")).unwrap()
	}
}

impl ssh2::KeyboardInteractivePrompt for KeyboardInteration
{
	fn prompt<'a>(&mut self, username:&str, instructions: &str, prompts: &[ssh2::Prompt<'a>]) -> Vec<String>
	{
		println!("Asking username {} for its password. {}",username,instructions);
		println!("{} prompts?",prompts.len());
		stdout().lock().flush().unwrap();
		//let line = stdin.lock().lines().next().unwrap().unwrap();
		//let mut line = String::new();
		//stdin().lock().read_line(&mut line).unwrap();
		//vec![line]
		vec![rpassword::read_password_from_tty(Some("Password: ")).unwrap()]
	}
}

///Collect the output of
///		$ squeue -ho '%A'
///into a vector.
fn gather_slurm_jobs() -> Result<Vec<usize>,std::io::Error>
{
	let squeue_output=Command::new("squeue")
		//.current_dir(directory)
		.arg("-ho")
		.arg("%A")
		.output()?;
	let squeue_output=String::from_utf8_lossy(&squeue_output.stdout);
	Ok(squeue_output.lines().map(|line|
		//line.parse::<usize>().expect("squeue should give us integers")
		match line.parse::<usize>()
		{
			Ok(x) => x,
			Err(e) =>
			{
				panic!("error {} on parsing line [{}]",e,line);
			},
		}
	).collect())
}

///Simulations to be run in a slurm/other job.
struct Job
{
	execution_code_vec: Vec<String>,
	execution_id_vec: Vec<usize>,
}

impl Job
{
	fn new()->Job
	{
		Job{
			execution_code_vec:vec![],
			execution_id_vec:vec![],
		}
	}

	fn len(&self)->usize
	{
		self.execution_id_vec.len()
	}

	fn add_execution(&mut self, execution_id: usize, binary:&Path, execution_path_str: &str)
	{
		let job_line=format!("echo execution {}\n/bin/date\n{} {}/local.cfg --results={}/local.result",execution_id,binary.display(),execution_path_str,execution_path_str);
		self.execution_code_vec.push(job_line);
		self.execution_id_vec.push(execution_id);
	}

	fn write_slurm_script(&self, out:&mut dyn Write,prefix:&str,slurm_time:&str, slurm_mem:Option<&str>,job_lines:&str)
	{
		// #SBATCH --mem=1000 ?? In megabytes or suffix [K|M|G|T]. See sbatch man page for more info.
		let mem_str = if let Some(s)=slurm_mem { format!("#SBATCH --mem={}\n",s) } else {"".to_string()};
		writeln!(out,"#!/bin/bash
#SBATCH --job-name=CAMINOS
#SBATCH -D .
#SBATCH --output={prefix}-%j.out
#SBATCH --error={prefix}-%j.err
#SBATCH --cpus-per-task=1
#SBATCH --ntasks=1
#SBATCH --time={slurm_time}
{mem_str}
{job_lines}
",prefix=prefix,slurm_time=slurm_time,mem_str=mem_str,job_lines=job_lines).unwrap();
	}

	fn launch_slurm_script(&self, directory:&Path,script_name:&str) -> usize
	{
		let sbatch_output=Command::new("sbatch")
			.current_dir(directory)
			.arg(script_name)
			.output()
			.expect("sbatch failed to start");
		//Should be something like "Submitted batch job 382683"
		let mut jobids=vec![];
		//let sbatch_stdout=sbatch_output.stdout.iter().collect::<String>();
		let sbatch_stdout=String::from_utf8_lossy(&sbatch_output.stdout);
		//for word in sbatch_stdout.split(" ")
		for word in sbatch_stdout.split_whitespace()
		{
			match word.parse::<usize>()
			{
				Ok(id) => jobids.push(id),
				Err(_) => (),
			};
		}
		if jobids.len()!=1
		{
			panic!("sbatch executed but we got incorrect jobids ({:?} from {})",jobids,sbatch_stdout);
		}
		jobids[0]
	}
	
	///Creates a slurm script with the jobs and launch them. Returns a description to include in the journal.
	///internal_job_id is the one used in the script files. Currently being the id of the first experiment in the batch.
	///jobs_path is the path where the launch script is created.
	///slurm_time and slurm_mem are passed as slurm arguments.
	fn slurm(&mut self, internal_job_id:usize, jobs_path:&Path, slurm_time:&str, slurm_mem:Option<&str>) -> String
	{
		let job_lines=self.execution_code_vec.join("\n") + "\n/bin/date\necho job finished\n";
		let launch_name=format!("launch{}",internal_job_id);
		let launch_script=jobs_path.join(&launch_name);
		let mut launch_script_file=File::create(&launch_script).expect("Could not create launch file");
		self.write_slurm_script(&mut launch_script_file,&launch_name,slurm_time,slurm_mem,&job_lines);
		let slurm_job_id=self.launch_slurm_script(&jobs_path,&launch_name);
		//FIXME: we also need the execution ids inside that job.
		//let execution_id_string=self.execution_id_vec.join(",");
		//let execution_id_string=self.execution_id_vec.iter().map(|id|format!("{}",id)).zip(repeat(",")).collect::<String>();
		let execution_id_string=self.execution_id_vec.iter().map(|id|format!("{}",id)).collect::<Vec<String>>().join(",");
		format!("{}={}[{}], ",slurm_job_id,internal_job_id,execution_id_string)
	}
}

///Options that may modifiy the performed action.
#[non_exhaustive]
#[derive(Default)]
pub struct ExperimentOptions
{
	///Bring matching results from another experiment directory.
	pub external_source: Option<PathBuf>,
	///Experiment index in which to start the actions.
	pub start_index: Option<usize>,
	///Experiment index in which to end the actions (excluded).
	pub end_index: Option<usize>,
	///Expression of expriments to be included.
	pub where_clause: Option<config_parser::Expr>,
	///A message to be written into the log.
	pub message: Option<String>,
}

///An `Experiment` object encapsulates the operations that are performed over a folder containing an experiment.
pub struct Experiment<'a>
{
	files: ExperimentFiles,
	//options: Matches,
	options: ExperimentOptions,
	journal: PathBuf,
	journal_index: usize,
	remote_files: Option<ExperimentFiles>,
	//remote_host: Option<String>,
	//remote_username: Option<String>,
	//remote_binary: Option<PathBuf>,
	//remote_root: Option<PathBuf>,
	//ssh2_session: Option<Session>,
	//remote_binary_results: Option<ConfigurationValue>,
	#[allow(dead_code)]
	visible_slurm_jobs: Vec<usize>,
	owned_slurm_jobs: Vec<usize>,
	experiments_on_slurm: Vec<usize>,
	plugs:&'a Plugs,
}

///Each experiment owns some files:
/// * main.cfg
/// * main.od
/// * remote
/// * launch in the future, instead of being inside main.cfg
/// * runs/{run#,job#}
/// * binary.results
/// This does not implement Debug because Session does not...
pub struct ExperimentFiles
{
	///The host with the path of these files.
	///None if it is the hosting where this instance of caminos is running.
	host: Option<String>,
	///Optional username to access the host.
	username: Option<String>,
	///Whether we have a ssh2 session openened with that host.
	ssh2_session: Option<Session>,
	//TODO: learn what happens when paths are not UNICODE.
	//TODO: perhaps it should be possible a ssh:// location. Maybe an URL.
	///The path where caminos binary file is located.
	binary: Option<PathBuf>,
	///The root path of the experiments
	root: Option<PathBuf>,
	///The raw contents of the main.cfg file
	cfg_contents: Option<String>,
	///
	parsed_cfg: Option<config_parser::Token>,
	runs_path: Option<PathBuf>,
	///The experiments as extracted from the main.cfg.
	experiments: Vec<ConfigurationValue>,
	///The list of configurations for launch.
	///Either extracted from main.cfg field `launch_configurations`
	/// or from the launch file (TODO the latter).
	launch_configurations: Vec<ConfigurationValue>,
	///The results packeted (or to be packeted) in binary.results.
	packed_results: ConfigurationValue,
}

impl ExperimentFiles
{
	pub fn build_cfg_contents(&mut self)
	{
		if let None = self.cfg_contents
		{
			let cfg=self.root.as_ref().unwrap().join("main.cfg");
			if let Some(session) = &self.ssh2_session {
				let sftp = session.sftp().expect("error starting sftp");
				let mut remote_main_cfg =  sftp.open(&cfg).expect("Could not open remote main.cfg");
				let mut remote_main_cfg_contents=String::new();
				remote_main_cfg.read_to_string(&mut remote_main_cfg_contents).expect("Could not read remote main.cfg.");
				self.cfg_contents = Some(remote_main_cfg_contents);
			} else {
				let cfg_contents={
					let mut cfg_contents = String::new();
					let mut cfg_file=File::open(&cfg).expect("main.cfg could not be opened");
					cfg_file.read_to_string(&mut cfg_contents).expect("something went wrong reading main.cfg");
					cfg_contents
				};
				self.cfg_contents = Some(cfg_contents);
				//println!("cfg_contents={:?}",cfg_contents);
			}
		}
	}
	pub fn cfg_contents_ref(&self) -> &String
	{
		self.cfg_contents.as_ref().unwrap()
	}
	pub fn build_parsed_cfg(&mut self)
	{
		if let None = self.parsed_cfg
		{
			self.build_cfg_contents();
			let parsed_cfg=match config_parser::parse(self.cfg_contents_ref())
			{
				Err(x) => panic!("error parsing configuration file: {:?}",x),
				Ok(x) => x,
				//println!("parsed correctly: {:?}",x);
			};
			//println!("parsed_cfg={:?}",parsed_cfg);
			self.parsed_cfg = Some(parsed_cfg);
		}
	}
	pub fn build_runs_path(&mut self)
	{
		if let None = self.runs_path
		{
			let mut is_old=false;
			//for experiment_index in 0..experiments.len()
			//{
			//	let experiment_path=self.files.root.join(format!("run{}",experiment_index));
			//	if experiment_path.is_dir()
			//	{
			//		is_old=true;
			//		break;
			//	}
			//}
			if self.root.as_ref().unwrap().join("run0").is_dir()
			{
				is_old=true;
			}
			let runs_path = if is_old
			{
				self.root.as_ref().unwrap().join("")
			}
			else
			{
				let runs_path=self.root.as_ref().unwrap().join("runs");
				if !runs_path.is_dir()
				{
					fs::create_dir(&runs_path).expect("Something went wrong when creating the runs directory.");
				}
				runs_path
			};
			let runs_path=runs_path.canonicalize().unwrap_or_else(|e|panic!("The runs path \"{:?}\" cannot be resolved (error {})",runs_path,e));
			self.runs_path = Some( runs_path );
		}
	}
	pub fn build_experiments(&mut self)
	{
		self.build_parsed_cfg();
		self.experiments=match self.parsed_cfg
		{
			Some(config_parser::Token::Value(ref value)) =>
			{
				let flat=flatten_configuration_value(value);
				if let ConfigurationValue::Experiments(experiments)=flat
				{
					experiments
				}
				else
				{
					panic!("there are not experiments");
				}
			},
			_ => panic!("Not a value. Got {:?}",self.parsed_cfg),
		};
	}
	pub fn build_launch_configurations(&mut self)
	{
		self.build_parsed_cfg();
		if let config_parser::Token::Value(ref value)=self.parsed_cfg.as_ref().unwrap()
		{
			if let &ConfigurationValue::Object(ref cv_name, ref cv_pairs)=value
			{
				// Configuration {
				//  launch_configurations: [
				//    Slurm {...}
				//  ]
				//}
				if cv_name!="Configuration"
				{
					panic!("A simulation must be created from a `Configuration` object not `{}`",cv_name);
				}
				//let mut maximum_jobs=None;
				//let mut time:Option<&str> =None;
				//let mut mem =None;
				//let mut option_job_pack_size=None;
				for &(ref name,ref value) in cv_pairs
				{
					match name.as_ref()
					{
						"launch_configurations" => match value
						{
							&ConfigurationValue::Array(ref l) => self.launch_configurations = l.clone(),
							_ => panic!("bad value for launch_configurations"),
						}
						_ => (),
					}
				}
			}
			else
			{
				panic!("Those are not experiments.");
			}
		}
	}
	///Returns true if their main.cfg content is the same
	///Otherwise panics and prints a diff.
	pub fn compare_cfg(&self, other:&ExperimentFiles) -> bool
	{
		let local_content = self.cfg_contents.as_ref().unwrap();
		let other_content = other.cfg_contents.as_ref().unwrap();
		if local_content == other_content {
			println!("The configurations match");
			return true;
		} else {
			let mut last_both=None;
			let mut show_both=false;
			let mut count_left=0;
			let mut count_right=0;
			let mut show_count=true;
			for diff in diff::lines(local_content, other_content)
			{
				match diff {
					diff::Result::Left(x) =>
					{
						if show_count
						{
							println!("@left line {}, right line {}",count_left,count_right);
							show_count=false;
						}
						if let Some(p)=last_both.take()
						{
							println!(" {}",p);
						}
						println!("-{}",x);
						show_both=true;
						count_left+=1;
					},
					diff::Result::Right(x) =>
					{
						if show_count
						{
							println!("@left line {}, right line {}",count_left,count_right);
							show_count=false;
						}
						if let Some(p)=last_both.take()
						{
							println!(" {}",p);
						}
						println!("+{}",x);
						show_both=true;
						count_right+=1;
					},
					diff::Result::Both(x,_) =>
					{
						if show_both
						{
							println!(" {}",x);
							show_both=false;
						}
						last_both = Some(x);
						show_count=true;
						count_left+=1;
						count_right+=1;
					},
				}
			}
			let cfg = self.root.as_ref().unwrap().join("main.cfg");
			let remote_cfg_path = other.root.as_ref().unwrap().join("main.cfg");
			let username = other.username.as_ref().unwrap();
			let host = other.host.as_ref().unwrap();
			panic!("The configurations do not match.\nYou may try$ vimdiff {:?} scp://{}@{}/{:?}\n",cfg,username,host,remote_cfg_path);
		}
	}
	pub fn build_packed_results(&mut self)
	{
		let packed_results_path = self.root.as_ref().unwrap().join("binary.results");
		self.packed_results = if let Some(session) = &self.ssh2_session {
			match session.scp_recv(&packed_results_path)
			{
				Ok( (mut remote_binary_results_channel, _stat) ) => {
					let mut remote_binary_results_contents= vec![];
					remote_binary_results_channel.read_to_end(&mut remote_binary_results_contents).expect("Could not read remote binary.results");
					let got = config::config_from_binary(&remote_binary_results_contents,0).expect("something went wrong while deserializing binary.results");
					match got
					{
						ConfigurationValue::Experiments(ref _a) => {
							//We do not have the `experiments` list in here.
							//if a.len()!=n {
							//	panic!("The Experiments stored in binary.results has length {} instead of {} as the number of experiment items",a.len(),n);
							//}
						},
						_ => panic!("A non-Experiments stored on binary.results"),
					};
					got
				},
				Err(_) => ConfigurationValue::None,
			}
		} else {
			let n = self.experiments.len();
			match File::open(&packed_results_path)
			{
				Err(_) => {
					ConfigurationValue::Experiments( (0..n).map(|_|ConfigurationValue::None).collect() )
				},
				Ok(ref mut file) => {
					let mut contents = Vec::with_capacity(n);
					file.read_to_end(&mut contents).expect("something went wrong reading binary.results");
					let got = config::config_from_binary(&contents,0).expect("something went wrong while deserializing binary.results");
					match got
					{
						ConfigurationValue::Experiments(ref a) => {
							if a.len()!=n {
								panic!("The Experiments stored in binary.results has length {} instead of {} as the number of experiment items",a.len(),n);
							}
						},
						_ => panic!("A non-Experiments stored on binary.results"),
					};
					got
				},
			}
		};
	}
	pub fn example_cfg() -> &'static str
	{
		include_str!("defaults/main.cfg")
	}
	pub fn example_od() -> &'static str
	{
		include_str!("defaults/main.od")
	}
	pub fn example_remote() -> &'static str
	{
		include_str!("defaults/remote")
	}
}

impl<'a> Experiment<'a>
{
	///Creates a new experiment object.
	//pub fn new(binary:&Path,root:&Path,plugs:&'a Plugs,options:&Matches)->Experiment<'a>
	pub fn new(binary:&Path,root:&Path,plugs:&'a Plugs,options:ExperimentOptions)->Experiment<'a>
	{
		println!("Preparing experiment with {:?} as path",root);
		let visible_slurm_jobs:Vec<usize> = gather_slurm_jobs().unwrap_or(vec![]);
		let journal=root.join("journal");
		let journal_file=OpenOptions::new().read(true).write(true).create(true).open(&journal).expect("Something went wrong reading or creating the journal file");
		//let journal_len=journal_file.stream_len();
		//journal.file.seek(SeekFrom::End(0));
		let mut journal_index=0;
		let reader = BufReader::new(journal_file);
		let mut owned_slurm_jobs=vec![];
		let mut experiments_on_slurm=vec![];
		for rline in reader.lines()
		{
			//journal_index= rline.expect("bad line read from journal").split(":").next().expect("Not found the expected journal index").parse().expect("The journal index must be a non-negative integer");
			let line=rline.expect("bad line read from journal");
			if ! line.is_empty()
			{
				//let prefix=line.split(":").next().expect("Not found the expected journal index");
				let mut s = line.split(":");
				let prefix=s.next().expect("Not found the expected journal index");
				journal_index= 1usize+prefix.parse::<usize>().unwrap_or_else(|_|panic!("The journal index must be a non-negative integer (received {})",prefix));
				let entry = s.next().expect("No content found on the journal line");
				if entry.starts_with(" Launched jobs ")
				{
					//e.g:
					//	0: Launched jobs 457688=5[0,1,2,3,4,5], 457689=11[6,7,8,9,10,11], 457690=17[12,13,14,15,16,17], 457691=23[18,19,20,21,22,23],
					let mut slurm_items=entry.split(" ");
					slurm_items.next();//first empty space
					slurm_items.next();//Launched
					slurm_items.next();//jobs
					for slurm_item in slurm_items
					{
						if slurm_item.is_empty()
						{
							continue;
						}
						let mut slurm_pair = slurm_item.split("=");
						let slurm_job_id = slurm_pair.next().unwrap().parse::<usize>().unwrap_or_else(|_|panic!("left term on '{}' should be an integer",slurm_item));
						if visible_slurm_jobs.contains(&slurm_job_id)
						{
							owned_slurm_jobs.push(slurm_job_id);
							let slurm_job_content = slurm_pair.next().unwrap();
							let left_bracket_index = slurm_job_content.find("[").unwrap();
							let right_bracket_index = slurm_job_content.find("]").unwrap();
							let experiments=slurm_job_content[left_bracket_index+1 .. right_bracket_index].split(",").map(|item|item.parse::<usize>().unwrap_or_else(|_|panic!("failed with content={} for item {}",slurm_job_content,slurm_item)));
							experiments_on_slurm.extend(experiments);
						}
					}
				}
				if entry==" message"
				{
					println!("journal message {}",line);
				}
			}
		}
		Experiment{
			files: ExperimentFiles{
				host: None,
				username: None,
				ssh2_session: None,
				binary: Some(binary.to_path_buf()),
				root: Some(root.to_path_buf()),
				cfg_contents: None,
				parsed_cfg: None,
				runs_path: None,
				experiments: Vec::new(),
				launch_configurations: Vec::new(),
				packed_results: ConfigurationValue::None,
			},
			options,
			journal,
			journal_index,
			remote_files: None,
			//remote_host: None,
			//remote_username: None,
			//remote_binary: None,
			//remote_root: None,
			//ssh2_session: None,
			//remote_binary_results: None,
			visible_slurm_jobs,
			owned_slurm_jobs,
			experiments_on_slurm,
			plugs,
		}
	}
	/// Appends a new entry to the journal
	fn write_journal_entry(&self, entry:&str)
	{
		let mut journal_file=OpenOptions::new().append(true).open(&self.journal).expect("Something went wrong reading or creating the journal file");
		writeln!(journal_file,"{}: {}",self.journal_index,entry).expect("Could not write to journal");
	}
	/// Executes an action over the experiment.
	pub fn execute_action(&mut self,action:Action)
	{
		let now = chrono::Utc::now();
		self.write_journal_entry(&format!("Executing action {} on {}.", action, now.format("%Y %m(%b) %0d(%a), %T (UTC%:z)").to_string()));
		let cfg=self.files.root.as_ref().unwrap().join("main.cfg");
		//TODO cfg checkum
		//let mut cfg_contents = String::new();
		//let mut cfg_file=File::open(&cfg).expect("main.cfg could not be opened");
		//cfg_file.read_to_string(&mut cfg_contents).expect("something went wrong reading main.cfg");
		match action
		{
			Action::Shell => 
			{
				if cfg.exists()
				{
					panic!("{:?} already exists, could not proceed with the shell action. To generate new files delete main.cfg manually.",cfg);
				}
				if let Some(ref path) = self.options.external_source
				{
					//Copy files from the source path.
					fs::copy(path.join("main.cfg"),&cfg).expect("error copying main.cfg");
					fs::copy(path.join("main.od"),self.files.root.as_ref().unwrap().join("main.od")).expect("error copying main.od");
					//TODO: Try to update the paths in the remote file.
					fs::copy(path.join("remote"),self.files.root.as_ref().unwrap().join("remote")).expect("error copying remote");
				} else {
					//Write some default files.
					let mut new_cfg_file=File::create(&cfg).expect("Could not create main.cfg.");
					writeln!(new_cfg_file,"{}",ExperimentFiles::example_cfg()).unwrap();
					let mut new_od_file=File::create(self.files.root.as_ref().unwrap().join("main.od")).expect("Could not create main.od.");
					writeln!(new_od_file,"{}",ExperimentFiles::example_od()).unwrap();
					let mut new_remote_file=File::create(self.files.root.as_ref().unwrap().join("remote")).expect("Could not create remote.");
					writeln!(new_remote_file,"{}",ExperimentFiles::example_remote()).unwrap();
				};
			},
			_ => (),
		}
		let mut results;
		self.files.build_experiments();

		let external_files = if let (Some(ref path),true) = (self.options.external_source.as_ref(), action!=Action::Shell  ) {
			let mut ef = ExperimentFiles{
				host: None,
				username: None,
				ssh2_session: None,
				binary: None,
				root: Some(path.to_path_buf()),
				cfg_contents: None,
				parsed_cfg: None,
				runs_path: None,
				experiments: Vec::new(),
				launch_configurations: Vec::new(),
				packed_results: ConfigurationValue::None,
			};
			ef.build_experiments();
			ef.build_packed_results();
			Some(ef)
		} else {
			None
		};
		//let (external_experiments,external_binary_results) = if let (Some(ref path),true) = (self.options.external_source.as_ref(), action!=Action::Shell  )
		//{
		//	let cfg = path.join("main.cfg");
		//	let mut cfg_file=File::open(&cfg).unwrap_or_else(|_|panic!("main.cfg from --source={:?} could not be opened",path));
		//	let mut cfg_contents = String::new();
		//	cfg_file.read_to_string(&mut cfg_contents).unwrap_or_else(|_|panic!("something went wrong reading main.cfg from --source={:?}",path));
		//	let parsed_cfg=match config_parser::parse(&cfg_contents)
		//	{
		//		Err(x) => panic!("error parsing configuration file: {:?} from --source={:?}",x,path),
		//		Ok(x) => x,
		//		//println!("parsed correctly: {:?}",x);
		//	};
		//	//Some(parsed_cfg)
		//	let experiments = match parsed_cfg
		//	{
		//		config_parser::Token::Value(ref value) =>
		//		{
		//			let flat=flatten_configuration_value(value);
		//			if let ConfigurationValue::Experiments(experiments)=flat
		//			{
		//				experiments
		//			}
		//			else
		//			{
		//				panic!("there are not experiments in --source={:?}",path);
		//			}
		//		},
		//		_ => panic!("Not a value in --cource={:?}",path),
		//	};
		//	let packed_results_path = path.join("binary.results");
		//	let packed_results = {
		//		let n = experiments.len();
		//		match File::open(&packed_results_path)
		//		{
		//			Err(_) => {
		//				println!("Error opening external binary.results");
		//				//ConfigurationValue::Experiments( (0..n).map(|_|ConfigurationValue::None).collect() )
		//				None
		//			},
		//			Ok(ref mut file) => {
		//				let mut contents = Vec::with_capacity(n);
		//				file.read_to_end(&mut contents).expect("something went wrong reading binary.results");
		//				let got = config::config_from_binary(&contents,0).expect("something went wrong while deserializing binary.results");
		//				match got
		//				{
		//					ConfigurationValue::Experiments(ref a) => {
		//						if a.len()!=n {
		//							panic!("The Experiments stored in binary.results has length {} instead of {} as the number of experiment items",a.len(),n);
		//						}
		//					},
		//					_ => panic!("A non-Experiments stored on binary.results"),
		//				};
		//				Some(got)
		//			},
		//		}
		//	};
		//	(Some(experiments),packed_results)
		//} else {(None,None)};
		
		if let Some(message)=&self.options.message
		{
			self.write_journal_entry(&format!("message: {}",message));
		}

		self.files.build_packed_results();
		let mut added_packed_results = 0usize;

		let mut must_draw=false;
		let mut job_pack_size=1;//how many binary runs per job.
		//let mut pending_jobs=vec![];
		let mut job=Job::new();
		let mut slurm_time : String = "0-24:00:00".to_string();
		let mut slurm_mem: Option<String>=None;
		let mut uses_jobs=false;
		match action
		{
			Action::LocalAndOutput =>
			{
				must_draw=true;
			},
			Action::Local =>
			{
				must_draw=false;
			},
			Action::Output =>
			{
				must_draw=true;
			},
			Action::Slurm =>
			{
				uses_jobs=true;
				let n = self.files.experiments.len();
						
				let mut maximum_jobs=None;
				let mut time:Option<&str> =None;
				let mut mem =None;
				let mut option_job_pack_size=None;
				self.files.build_launch_configurations();
				for lc in self.files.launch_configurations.iter()
				{
					match lc
					{
						&ConfigurationValue::Object(ref launch_name, ref launch_pairs) =>
						{
							if launch_name=="Slurm"
							{
								for &(ref slurm_name,ref slurm_value) in launch_pairs
								{
									match slurm_name.as_ref()
									{
										"maximum_jobs" => match slurm_value
										{
											&ConfigurationValue::Number(f) => maximum_jobs=Some(f as usize),
											_ => panic!("bad value for maximum_jobs"),
										}
										"job_pack_size" => match slurm_value
										{
											&ConfigurationValue::Number(f) => option_job_pack_size=Some(f as usize),
											_ => panic!("bad value for option_job_pack_size"),
										}
										"time" => match slurm_value
										{
											//&ConfigurationValue::Literal(ref s) => time=Some(&s[1..s.len()-1]),
											&ConfigurationValue::Literal(ref s) => time=Some(s.as_ref()),
											_ => panic!("bad value for time"),
										}
										"mem" => match slurm_value
										{
											//&ConfigurationValue::Literal(ref s) => mem=Some(&s[1..s.len()-1]),
											&ConfigurationValue::Literal(ref s) => mem=Some(s.as_ref()),
											_ => panic!("bad value for mem"),
										}
										_ => (),
									}
								}
							}
						}
						_ => (),//XXX perhaps error on unknown launch configuration?
					}
				}
				if let Some(value)=maximum_jobs
				{
					let new_job_pack_size=(n + value-1 ) / value;//rounding up of experiments/maximum
					if new_job_pack_size>=job_pack_size
					{
						job_pack_size=new_job_pack_size;
					}
					else
					{
						panic!("Trying to reduce job_pack_size from {} to {}.",job_pack_size,new_job_pack_size);
					}
				}
				if let Some(value)=option_job_pack_size
				{
					if job_pack_size!=1 && value!=1
					{
						panic!("Trying to change job_pack_size unexpectedly");
					}
					job_pack_size = value;
				}
				if let Some(value)=time
				{
					slurm_time=value.to_string();
				}
				slurm_mem=mem.map(|x:&str|x.to_string());
			},
			Action::Check =>
			{
				must_draw=false;
			},
			Action::Pull =>
			{
				self.initialize_remote();
				self.remote_files.as_mut().unwrap().build_cfg_contents();
				self.files.compare_cfg(&self.remote_files.as_ref().unwrap());
			},
			Action::RemoteCheck =>
			{
				self.initialize_remote();
				let remote_root=self.remote_files.as_ref().unwrap().root.clone();
				let remote_binary=self.remote_files.as_ref().unwrap().binary.clone();
				let mut channel = self.remote_files.as_ref().unwrap().ssh2_session.as_ref().unwrap().channel_session().unwrap();
				let remote_command = format!("{:?} {:?} --action=check",remote_binary,remote_root);
				channel.exec(&remote_command).unwrap();
				let mut remote_command_output = String::new();
				channel.read_to_string(&mut remote_command_output).unwrap();
				channel.wait_close().expect("Could not close the channel of remote executions.");
				channel.exit_status().unwrap();
				for line in remote_command_output.lines()
				{
					println!("at remote: {}",line);
				}
			},
			Action::Push =>
			{
				self.initialize_remote();
				//Bring the remote files to this machine
				let remote_root=self.remote_files.as_ref().unwrap().root.clone().unwrap();
				//Download remote main.cfg
				let remote_cfg_path = remote_root.join("main.cfg");
				let sftp = self.remote_files.as_ref().unwrap().ssh2_session.as_ref().unwrap().sftp().unwrap();
				match sftp.stat(&remote_root)
				{
					Ok(remote_stat) =>
					{
						if !remote_stat.is_dir()
						{
							panic!("remote {:?} exists, but is not a directory",&remote_stat);
						}
						self.remote_files.as_mut().unwrap().build_cfg_contents();
						self.files.compare_cfg(&self.remote_files.as_ref().unwrap());
					},
					Err(_err) =>
					{
						println!("Could not open remote '{:?}', creating it",remote_root);
						sftp.mkdir(&remote_root,0o755).expect("Could not create remote directory");
						let mut remote_cfg = sftp.create(&remote_cfg_path).expect("Could not create remote main.cfg");
						write!(remote_cfg,"{}",self.files.cfg_contents_ref()).expect("Could not write into remote main.cfg");
						let mut remote_od = sftp.create(&remote_root.join("main.od")).expect("Could not create remote main.od");
						let mut local_od = File::open(self.files.root.as_ref().unwrap().join("main.od")).expect("Could not open local main.od");
						let mut od_contents = String::new();
						local_od.read_to_string(&mut od_contents).expect("something went wrong reading main.od");
						write!(remote_od,"{}",od_contents).expect("Could not write into remote main.od");
					},
				};
			},
			Action::SlurmCancel =>
			{
				//Cancel all jobs on owned_slurm_jobs
				let mut scancel=&mut Command::new("scancel");
				for jobid in self.owned_slurm_jobs.iter()
				{
					scancel = scancel.arg(jobid.to_string());
				}
				scancel.output().expect("scancel failed");
			},
			Action::Shell => (),
			Action::Pack => (),
		};

		//Remove mutabiity to prevent mistakes.
		let must_draw=must_draw;
		let job_pack_size=job_pack_size;
		let slurm_time=slurm_time;
		let slurm_mem=slurm_mem;
		let uses_jobs=uses_jobs;

		self.files.build_runs_path();
		let runs_path : PathBuf = self.files.runs_path.as_ref().unwrap().to_path_buf();

		//Execute or launch jobs.
		let start_index = self.options.start_index.unwrap_or(0);
		//if start_index<0 {panic!("start_index={} < 0",start_index);}
		if start_index>self.files.experiments.len() {panic!("start_index={} > experiments.len()={}",start_index,self.files.experiments.len());}
		let end_index = self.options.end_index.unwrap_or(self.files.experiments.len());
		//if end_index<0 {panic!("end_index={} < 0",end_index);}
		if end_index>self.files.experiments.len() {panic!("end_index={} > experiments.len()={}",end_index,self.files.experiments.len());}
		let jobs_path=runs_path.join(format!("jobs{}",self.journal_index));
		let mut launch_entry="".to_string();
		if uses_jobs && !jobs_path.is_dir()
		{
			fs::create_dir(&jobs_path).expect("Something went wrong when creating the jobs directory.");
		}
		let mut before_amount_completed=0;//We have a good local.result.
		let before_amount_slurm=self.experiments_on_slurm.len();//We can see the slurm job id in squeue. (and looking the journal file)
		let mut before_amount_inactive=0;//We have not done anything with the execution yet, i.e., no local.result.
		let mut before_amount_active=0;//We have a local.result with size 0, so we have done something. Perhaps some execution error.
		let mut delta_amount_slurm=0;
		let mut delta_completed=0;
		let sftp = self.remote_files.as_ref().map(|f|f.ssh2_session.as_ref().unwrap().sftp().unwrap());
		let progress_bar = ProgressBar::new((end_index-start_index) as u64);
		progress_bar.set_style(ProgressStyle::default_bar().template("{prefix} [{elapsed_precise}] {bar:30.blue/white.dim} {pos:5}/{len:5} {msg}"));
		let mut pulled=0;
		let mut empty=0;
		let mut missing=0;
		let mut merged=0;
		match action
		{
			Action::Pull => progress_bar.set_prefix("pulling files"),
			Action::Local | Action::LocalAndOutput => progress_bar.set_prefix("running locally"),
			Action::Slurm => progress_bar.set_prefix("preparing slurm scripts"),
			_ => progress_bar.set_prefix("checking result files"),
		};
		for (experiment_index,experiment) in self.files.experiments.iter().enumerate().skip(start_index).take(end_index-start_index)
		{
			progress_bar.inc(1);
			if let Some(ref expr) = self.options.where_clause
			{
				match evaluate(&expr,experiment,&self.files.root.as_ref().unwrap())
				{
					ConfigurationValue::True => (),//good
					ConfigurationValue::False => continue,//discard this index
					x => panic!("The where clause evaluate to a non-bool type ({:?})",x),
				}
			}
			let experiment_path=runs_path.join(format!("run{}",experiment_index));
			if !experiment_path.is_dir()
			{
				//Only some actions need to have the run folders.
				//Perhaps we could define a method to made them on demand.
				use Action::*;
				match action
				{
					Local|LocalAndOutput|Slurm => fs::create_dir(&experiment_path).expect("Something went wrong when creating the run directory."),
					_ => (),
				}
			}
			let is_packed = if let ConfigurationValue::Experiments(ref a) = self.files.packed_results {
				match a[experiment_index]
				{
					ConfigurationValue::None => false,
					_ => true,
				}
			} else {false};
			let result_path=experiment_path.join("local.result");
			//FIXME: check if the run is expected to be currently inside some slurm job.
			let has_file = result_path.is_file();
			let has_content=if !has_file
			{
				before_amount_inactive+=1;
				false
			}
			else
			{
				result_path.metadata().unwrap().len()>=5
			};
			let mut is_merged = false;
			if !has_content && !is_packed
			{
				//In all actions bring up experiments from the external_source if given.
				//if let Some(ref external_experiment_list) = external_experiments
				if let Some(ref external_files) = external_files
				{
					for (ext_index,ext_experiment) in external_files.experiments.iter().enumerate()
					{
						if experiment==ext_experiment
						{
							//println!("matching local experiment {} with external experiment {}",experiment_index,ext_index);
							let mut ext_result_contents=None;
							let mut ext_result_value:Option<ConfigurationValue> = None;
							if let ConfigurationValue::Experiments(ref a) = external_files.packed_results
							{
								//println!("got {:?}", a[ext_index]);
								ext_result_value = Some( a[ext_index].clone() );
								//println!("external data in binary");
							} else {
								let ext_path=self.options.external_source.as_ref().unwrap().join(format!("runs/run{}/local.result",ext_index));
								let mut ext_result_file=match File::open(&ext_path)
								{
									Ok(rf) => rf,
									Err(_error) =>
									{
										//panic!("There are problems opening results (external experiment {}).",ext_index);
										continue;
									}
								};
								let mut aux=String::new();
								//remote_result_channel.read_to_string(&mut aux);
								ext_result_file.read_to_string(&mut aux).expect("Could not read remote result file.");
								if aux.len()>=5
								{
									ext_result_contents = Some ( aux );
								}
							}
							//println!("external data file:{} value:{}",ext_result_contents.is_some(),ext_result_value.is_some());
							if ext_result_contents.is_some() || ext_result_value.is_some()
							{
								//create file
								if let ConfigurationValue::Experiments(ref mut a) = self.files.packed_results
								{
									if ext_result_value.is_none()
									{
										if let Some(ref contents) = ext_result_contents
										{
											match config_parser::parse(&contents)
											{
												Ok(cv) =>
												{
													let result=match cv
													{
														config_parser::Token::Value(value) => value,
														_ => panic!("wrong token"),
													};
													ext_result_value = Some(result);
												}
												Err(_error)=>
												{
													println!("pulled invalid results (experiment {}).",experiment_index);
												}
											}
										}
									}
									a[experiment_index] = ext_result_value.unwrap();
									added_packed_results+=1;
								}
								else
								{
									//create file
									if ext_result_contents.is_none()
									{
										ext_result_contents = Some(format!("{}",ext_result_value.as_ref().unwrap()));
									}
									let mut new_result_file=File::create(&result_path).expect("Could not create result file.");
									writeln!(new_result_file,"{}",ext_result_contents.unwrap()).unwrap();
									//drop(new_result_file);//ensure it closes and syncs
								}
								merged+=1;
								is_merged=true;
							}
						}
					}
				}
			}
			if let (true,Action::Pack) =  (has_content,action)
			{
				let mut result_file=match File::open(&result_path)
				{
					Ok(rf) => rf,
					Err(_error) =>
					{
						//println!("There are problems opening results (experiment {}).",experiment_index);
						continue;
					}
				};
				let mut result_contents=String::new();
				result_file.read_to_string(&mut result_contents).expect("something went wrong reading the result file.");
				let result = match config_parser::parse(&result_contents)
				{
					Ok(cv) =>
					{
						match cv
						{
							config_parser::Token::Value(value) => value,
							_ => panic!("wrong token"),
						}
					}
					Err(_error)=>
					{
						println!("There are missing results (experiment {}).",experiment_index);
						ConfigurationValue::None
					}
				};
				if let ConfigurationValue::Experiments(ref mut a) = self.files.packed_results
				{
					match a[experiment_index]
					{
						ConfigurationValue::None =>
						{
							//It is not currently packed, so we write it.
							a[experiment_index] = result;
							added_packed_results+=1;
						},
						_ =>
						{
							//There is a current packed version. We check it is the same.
							if a[experiment_index] != result
							{
								panic!("Packed mistmatch at experiment index {}",experiment_index);
							}
						},
					};
				} else { panic!("broken pack"); }
			}
			//if !result_path.is_file() || result_path.metadata().unwrap().len()==0
			if has_content || is_packed || is_merged
			{
				before_amount_completed+=1;
				progress_bar.set_message(&format!("{} pulled, {} empty, {} missing, {} already, {} merged",pulled,empty,missing,before_amount_completed,merged));
			}
			else
			{
				if has_file
				{
					before_amount_active+=1;
				}
				match action
				{
					Action::Local | Action::LocalAndOutput =>
					{
						println!("experiment {} of {} is {:?}",experiment_index,self.files.experiments.len(),experiment);
						let mut simulation=Simulation::new(&experiment,self.plugs);
						simulation.run();
						simulation.write_result(&mut File::create(&result_path).expect("Could not create the result file."));
					},
					Action::Slurm => if !self.experiments_on_slurm.contains(&experiment_index)
					{
						let real_experiment_path=experiment_path.canonicalize().expect("This path cannot be resolved");
						let experiment_path_string = real_experiment_path.to_str().expect("You should use paths representable with unicode");
						let local_cfg=experiment_path.join("local.cfg");
						let mut local_cfg_file=File::create(&local_cfg).expect("Could not create local.cfg file");
						writeln!(local_cfg_file,"{}",experiment).unwrap();
						//let job_line=format!("echo experiment {}\n/bin/date\n{} {}/local.cfg --results={}/local.result",experiment_index,self.binary.display(),experiment_path_string,experiment_path_string);
						//pending_jobs.push(job_line);
						job.add_execution(experiment_index,&self.files.binary.as_ref().unwrap(),&experiment_path_string);
						if job.len()>=job_pack_size
						{
							delta_amount_slurm+=job.len();
							let job_id=experiment_index;
							let slurm_mem : Option<&str> = match slurm_mem { Some(ref x) => Some(x), None=>None };
							launch_entry += &job.slurm(job_id,&jobs_path,slurm_time.as_ref(),slurm_mem);
							job=Job::new();
						}
					},
					Action::Pull =>
					{
						let (remote_result,remote_result_contents) = 
						{
							self.remote_files.as_mut().unwrap().build_packed_results();
							let binary_result = match self.remote_files.as_ref().unwrap().packed_results{
								ConfigurationValue::Experiments(ref a) => if let ConfigurationValue::None = a[experiment_index] { None } else { Some(a[experiment_index].clone()) },
								ConfigurationValue::None => None,
								 _  => panic!("remote binary.results is corrupted"),
							};
							match binary_result
							{
								Some(x)=> (Some(x),None),
								None => {
									//println!("Could not open results of experiment {}, trying to pull it.",experiment_index);
									//println!("Trying to pull experiment {}.",experiment_index);
									//let session = self.ssh2_session.as_ref().unwrap();
									let remote_root=self.remote_files.as_ref().unwrap().root.clone().unwrap();
									let remote_result_path = remote_root.join(format!("runs/run{}/local.result",experiment_index));
									//let (mut remote_result_channel, stat) = match session.scp_recv(&remote_result_path)
									//{
									//	Ok( value ) => value,
									//	Err( _ ) =>
									//	{
									//		println!("Could not pull {}, skipping it",experiment_index);
									//		continue;
									//	},
									//};
									let mut remote_result_file = match sftp.as_ref().unwrap().open(&remote_result_path)
									{
										Ok(file) => file,
										Err(_err) =>
										{
											//println!("could not read remote file ({}).",err);
											missing+=1;
											continue;
										}
									};
									let mut remote_result_contents=String::new();
									//remote_result_channel.read_to_string(&mut remote_result_contents);
									remote_result_file.read_to_string(&mut remote_result_contents).expect("Could not read remote result file.");
									if remote_result_contents.len()<5
									{
										//println!("Remote file does not have contents.");
										empty+=1;
										(None,Some(remote_result_contents))
									} else {
										match config_parser::parse(&remote_result_contents)
										{
											Ok(cv) =>
											{
												let result=match cv
												{
													config_parser::Token::Value(value) => value,
													_ => panic!("wrong token"),
												};
												(Some(result),Some(remote_result_contents))
											}
											Err(_error)=>
											{
												println!("pulled invalid results (experiment {}).",experiment_index);
												(None,None)
											}
										}
									}
								},
							}
						};
						if let Some(result) = remote_result
						{
							if let ConfigurationValue::Experiments(ref mut a) = self.files.packed_results
							{
								a[experiment_index] = result;
								added_packed_results+=1;
							}
							else
							{
								//create file
								let remote_result_contents = match remote_result_contents
								{
									Some(x) => x,
									None => format!("{}",result),
								};
								let mut new_result_file=File::create(&result_path).expect("Could not create result file.");
								writeln!(new_result_file,"{}",remote_result_contents).unwrap();
								//drop(new_result_file);//ensure it closes and syncs
							}
							delta_completed+=1;
							pulled+=1;
						}
						//File::open(&result_path).expect("did not work even after pulling it.")
						progress_bar.set_message(&format!("{} pulled, {} empty, {} missing, {} already, {} merged",pulled,empty,missing,before_amount_completed,merged));
					}
					Action::Output | Action::Check | Action::RemoteCheck | Action::Push | Action::SlurmCancel | Action::Shell | Action::Pack =>
					{
					},
				};
			}
		}
		progress_bar.finish();
		if job.len()>0
		{
			let job_id=self.files.experiments.len();
			let slurm_mem : Option<&str> = match slurm_mem { Some(ref x) => Some(x), None=>None };
			launch_entry += &job.slurm(job_id,&jobs_path,slurm_time.as_ref(),slurm_mem);
			drop(job);
		}

		if ! launch_entry.is_empty()
		{
			self.write_journal_entry(&format!("Launched jobs {}",launch_entry));
		}

		let status_string = format!("Before: completed={} of {} slurm={} inactive={} active={} Changed: slurm=+{} completed=+{}",before_amount_completed,self.files.experiments.len(),before_amount_slurm,before_amount_inactive,before_amount_active,delta_amount_slurm,delta_completed);
		self.write_journal_entry(&status_string);
		println!("{}",status_string);
		
		if must_draw
		{
			results=Vec::with_capacity(self.files.experiments.len());
			//for (experiment_index,experiment) in experiments.iter().enumerate()
			for (experiment_index,experiment) in self.files.experiments.iter().enumerate().skip(start_index).take(end_index-start_index)
			{
				if let ConfigurationValue::Experiments(ref a) = self.files.packed_results
				{
					match &a[experiment_index]
					{
						&ConfigurationValue::None => (),
						result => {
							results.push((experiment_index,experiment.clone(),result.clone()));
							continue;
						},
					}
				}
				let experiment_path=runs_path.join(format!("run{}",experiment_index));
				let result_path=experiment_path.join("local.result");
				let mut result_file=match File::open(&result_path)
				{
					Ok(rf) => rf,
					Err(_error) =>
					{
						//println!("There are problems opening results (experiment {}).",experiment_index);
						continue;
					}
				};
				let mut result_contents=String::new();
				result_file.read_to_string(&mut result_contents).expect("something went wrong reading the result file.");
				//println!("result file read into a String");
				match config_parser::parse(&result_contents)
				{
					Ok(cv) =>
					{
						let result=match cv
						{
							config_parser::Token::Value(value) => value,
							_ => panic!("wrong token"),
						};
						if let ConfigurationValue::Experiments(ref mut a) = self.files.packed_results
						{
							a[experiment_index] = result.clone();
							added_packed_results+=1;
						}
						results.push((experiment_index,experiment.clone(),result));
					}
					Err(_error)=>
					{
						println!("There are missing results (experiment {}).",experiment_index);
					}
				}
				//println!("result file processed.");
			}
			//let binary_results_path = self.files.root.join("binary.results");
			//let mut binary_results_file=File::create(&binary_results_path).expect("Could not create binary results file.");
			//let full_results = ConfigurationValue::Experiments(results.iter().map(|(_cfg,res)|res.clone()).collect());
			//let binary_results = config::config_to_binary(&full_results).expect("error while serializing into binary");
			//binary_results_file.write_all(&binary_results).expect("error happened when creating binary file");

			//println!("results={:?}",results);
			let od=self.files.root.as_ref().unwrap().join("main.od");
			let mut od_file=File::open(&od).expect("main.od could not be opened");
			let mut od_contents = String::new();
			od_file.read_to_string(&mut od_contents).expect("something went wrong reading main.od");
			match config_parser::parse(&od_contents)
			{
				Err(x) => panic!("error parsing output description file: {:?}",x),
				Ok(config_parser::Token::Value(ConfigurationValue::Array(ref descriptions))) => for description in descriptions.iter()
				{
					println!("description={}",description);
					match create_output(&description,&results,self.files.experiments.len(),&self.files.root.as_ref().unwrap())
					{
						Ok(_) => (),
						Err(err) => println!("ERROR: could not create output {:?}",err),
					}
				},
				_ => panic!("The output description file does not contain a list.")
			};
		}
		if added_packed_results>=1
		{
			let packed_results_path = self.files.root.as_ref().unwrap().join("binary.results");
			let mut binary_results_file=File::create(&packed_results_path).expect("Could not create binary results file.");
			let binary_results = config::config_to_binary(&self.files.packed_results).expect("error while serializing into binary");
			binary_results_file.write_all(&binary_results).expect("error happened when creating binary file");
			println!("Added {} results to binary.results.",added_packed_results);
		}
		if let (Action::Pack,ConfigurationValue::Experiments(ref a)) = (action,&self.files.packed_results)
		{
			//Erase the raw results. After we have written correctly the binary file.
			for (experiment_index,value) in a.iter().enumerate()
			{
				match value
				{
					//If we do not have the result do not erase anything.
					ConfigurationValue::None => (),
					_ =>
					{
						let experiment_path=runs_path.join(format!("run{}",experiment_index));
						if experiment_path.exists()
						{
							if !experiment_path.is_dir()
							{
								panic!("Somehow {:?} exists but is not a directory",experiment_path);
							}
							fs::remove_dir_all(&experiment_path).unwrap_or_else(|e|panic!("Error {} when removing directory {:?} and its contents",e,experiment_path));
						}
					}
				}
			}
		}
		let fin = format!("Finished action {} on {}.", action, now.format("%Y %m(%b) %0d(%a), %T (UTC%:z)").to_string());
		self.write_journal_entry(&fin);
		println!("{}",fin);
	}
	///Tries to initiate a ssh session with the remote host.
	///Will ask a pasword via keyboard.
	fn initialize_remote(&mut self)
	{
		let remote_path = self.files.root.as_ref().unwrap().join("remote");
		let mut remote_file = File::open(&remote_path).expect("remote could not be opened");
		let mut remote_contents = String::new();
		remote_file.read_to_string(&mut remote_contents).expect("something went wrong reading remote.");
		let parsed_remote=match config_parser::parse(&remote_contents)
		{
			Err(x) => panic!("error parsing remote file: {:?}",x),
			Ok(x) => x,
			//println!("parsed correctly: {:?}",x);
		};
		match parsed_remote
		{
			config_parser::Token::Value(ref value) =>
			{
				if let ConfigurationValue::Array(ref l)=value
				{
					for remote_value in l
					{
						let mut name:Option<String> = None;
						let mut host:Option<String> = None;
						let mut username:Option<String> = None;
						let mut root:Option<String> = None;
						let mut binary:Option<String> = None;
						if let &ConfigurationValue::Object(ref cv_name, ref cv_pairs)=remote_value
						{
							if cv_name!="Remote"
							{
								panic!("A remote must be created from a `Remote` object not `{}`",cv_name);
							}
							for &(ref cvname,ref value) in cv_pairs
							{
								match cvname.as_ref()
								{
									"name" => match value
									{
										&ConfigurationValue::Literal(ref s) => name=Some(s.to_string()),
										_ => panic!("bad value for a remote name"),
									},
									"host" => match value
									{
										&ConfigurationValue::Literal(ref s) => host=Some(s.to_string()),
										_ => panic!("bad value for a remote host"),
									},
									"username" => match value
									{
										&ConfigurationValue::Literal(ref s) => username=Some(s.to_string()),
										_ => panic!("bad value for a remote username"),
									},
									"root" => match value
									{
										&ConfigurationValue::Literal(ref s) => root=Some(s.to_string()),
										_ => panic!("bad value for a remote root"),
									},
									"binary" => match value
									{
										&ConfigurationValue::Literal(ref s) => binary=Some(s.to_string()),
										_ => panic!("bad value for a remote binary"),
									},
									_ => panic!("Nothing to do with field {} in Remote",cvname),
								}
							}
						}
						else
						{
							panic!("Trying to create a remote from a non-Object");
						}
						if name==Some("default".to_string())
						{
							self.remote_files = Some(ExperimentFiles {
								host,
								username,
								ssh2_session: None,
								binary: binary.map(|value|Path::new(&value).to_path_buf()),
								root: root.map(|value|Path::new(&value).to_path_buf()),
								cfg_contents: None,
								parsed_cfg: None,
								runs_path: None,
								experiments: vec![],
								launch_configurations: Vec::new(),
								packed_results: ConfigurationValue::None,
							});
						}
					}
				}
				else
				{
					panic!("there are not remotes");
				}
			},
			_ => panic!("Not a value"),
		};
		//remote values are initialized
		let host=self.remote_files.as_ref().unwrap().host.as_ref().expect("there is no host").to_owned();
		//See ssh2 documentation https://docs.rs/ssh2/0.8.2/ssh2/index.html
		let tcp = TcpStream::connect(format!("{}:22",host)).unwrap();
		let mut session = Session::new().unwrap();
		session.set_tcp_stream(tcp);
		session.handshake().unwrap();
		//See portable-pty crate /src/ssh.rs for a good example on using ssh2.
		//session.userauth_agent("cristobal").unwrap();//FIXME: this fails, as it does not get any password.
		//session.userauth_password("cristobal","").unwrap();//This also fails, without asking
		let prompt = KeyboardInteration;
		//session.userauth_keyboard_interactive("cristobal",&mut prompt).unwrap();
		let username = self.remote_files.as_ref().unwrap().username.as_ref().expect("there is no username").to_owned();
		let raw_methods = session.auth_methods(&username).unwrap();
		let methods: HashSet<&str> = raw_methods.split(',').collect();
		println!("{} available authentication methods ({})",methods.len(),raw_methods);
		//if !session.authenticated() && methods.contains("publickey")
		if !session.authenticated() && methods.contains("password")
		{
			let password=prompt.ask_password(&username,&host);
			session.userauth_password(&username,&password).expect("Password authentication failed.");
		}
		//if !session.authenticated() && methods.contains("publickey")
		assert!(session.authenticated());
		self.remote_files.as_mut().unwrap().ssh2_session = Some(session);
		println!("ssh2 session created with remote host");
		self.remote_files.as_mut().unwrap().build_packed_results();
	}
}
