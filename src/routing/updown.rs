/*!

Implementation of general Up/Down-like routings.

* UpDown
* UpDownStar (struct ExplicitUpDown)

*/

use ::rand::{rngs::StdRng};
use std::cell::RefCell;
use crate::match_object_panic;
use crate::config_parser::ConfigurationValue;
use crate::routing::prelude::*;
use crate::topology::{Topology,NeighbourRouterIteratorItem,Location};
use crate::matrix::Matrix;
// use std::collections::HashMap;
use std::rc::Rc;
// use rand::Rng;
use crate::routing::RoutingTable;

///Use a shortest up/down path from origin to destination.
///The up/down paths are understood as provided by `Topology::up_down_distance`.
#[derive(Debug)]
pub struct UpDown
{
}

impl Routing for UpDown
{
	fn next(&self, _routing_info:&RoutingInfo, topology:&dyn Topology, current_router:usize, target_router: usize, target_server:Option<usize>, num_virtual_channels:usize, _rng: &mut StdRng) -> Result<RoutingNextCandidates,Error>
	{
		//let (target_location,_link_class)=topology.server_neighbour(target_server);
		//let target_router=match target_location
		//{
		//	Location::RouterPort{router_index,router_port:_} =>router_index,
		//	_ => panic!("The server is not attached to a router"),
		//};
		let (up_distance, down_distance) = topology.up_down_distance(current_router,target_router).unwrap_or_else(||panic!("The topology does not provide an up/down path from {} to {}",current_router,target_router));
		if up_distance + down_distance == 0
		{
			let target_server = target_server.expect("target server was not given.");
			for i in 0..topology.ports(current_router)
			{
				//println!("{} -> {:?}",i,topology.neighbour(current_router,i));
				if let (Location::ServerPort(server),_link_class)=topology.neighbour(current_router,i)
				{
					if server==target_server
					{
						//return (0..num_virtual_channels).map(|vc|(i,vc)).collect();
						//return (0..num_virtual_channels).map(|vc|CandidateEgress::new(i,vc)).collect();
						return Ok(RoutingNextCandidates{candidates:(0..num_virtual_channels).map(|vc|CandidateEgress::new(i,vc)).collect(),idempotent:true});
					}
				}
			}
			unreachable!();
		}
		let num_ports=topology.ports(current_router);
		let mut r=Vec::with_capacity(num_ports*num_virtual_channels);
		for i in 0..num_ports
		{
			//println!("{} -> {:?}",i,topology.neighbour(current_router,i));
			if let (Location::RouterPort{router_index,router_port:_},_link_class)=topology.neighbour(current_router,i)
			{
				if let Some((new_u, new_d)) = topology.up_down_distance(router_index,target_router)
				{
					if (new_u<up_distance && new_d<=down_distance) || (new_u<=up_distance && new_d<down_distance)
					{
						r.extend((0..num_virtual_channels).map(|vc|CandidateEgress::new(i,vc)));
					}
				}
			}
		}
		//println!("From router {} to router {} distance={} cand={}",current_router,target_router,distance,r.len());
		Ok(RoutingNextCandidates{candidates:r,idempotent:true})
	}
}

impl UpDown
{
	pub fn new(arg: RoutingBuilderArgument) -> UpDown
	{
		match_object_panic!(arg.cv,"UpDown",_value);
		UpDown{
		}
	}
}

///Use a shortest up/down path from origin to destination.
///But in contrast with UpDown this uses explicit table instead of querying the topology.
///Used to define Up*/Down* (UpDownStar), see Autonet, where it is build from some spanning tree.
/**
```ignore
UpDownStar{
	///The switch to select as root.
	root: 0,
	///Whether to allow travelling horizontal cross-branch links that reduce the up/down distance. Defaults to false.
	branch_crossing:true,
}
```
Note how the `branch_crossing` option would cause deadlock if it were allowed to use down-links. Consider three flows, each flow having
a unique posible last (down-link) hop. If this down-link could be used as a cross-branch by the next flow then that flow could block the former.
If this were to happen simultaneously with the three flows it would create a deadlock.
**/
#[derive(Debug)]
pub struct ExplicitUpDown
{
	//defining factors to be kept up to initialization
	pub root: Option<usize>,
	//computed at initialization
	pub up_down_distances: Matrix<Option<u8>>,
	pub down_distances: Matrix<Option<u8>>,
	pub distance_to_root: Vec<u8>,
	//other options
	pub branch_crossings_downwards: bool,
	pub branch_crossings_upwards: bool,
	pub label_up: i32,
	pub label_down: i32,
	pub label_horizontal_vec: Vec<i32>,
	pub label_horizontal_otherwise: i32,
}

impl Routing for ExplicitUpDown
{
	fn next(&self, _routing_info:&RoutingInfo, topology:&dyn Topology, current_router:usize, target_router: usize, target_server:Option<usize>, num_virtual_channels:usize, _rng: &mut StdRng) -> Result<RoutingNextCandidates,Error>
	{
		//let (target_location,_link_class)=topology.server_neighbour(target_server);
		//let target_router=match target_location
		//{
		//	Location::RouterPort{router_index,router_port:_} =>router_index,
		//	_ => panic!("The server is not attached to a router"),
		//};
		if current_router == target_router
		{
			let target_server = target_server.expect("target server was not given.");
			for i in 0..topology.ports(current_router)
			{
				//println!("{} -> {:?}",i,topology.neighbour(current_router,i));
				if let (Location::ServerPort(server),_link_class)=topology.neighbour(current_router,i)
				{
					if server==target_server
					{
						//return (0..num_virtual_channels).map(|vc|(i,vc)).collect();
						//return (0..num_virtual_channels).map(|vc|CandidateEgress::new(i,vc)).collect();
						return Ok(RoutingNextCandidates{candidates:(0..num_virtual_channels).map(|vc|CandidateEgress::new(i,vc)).collect(),idempotent:true});
					}
				}
			}
			unreachable!();
		}
		let up_down_distance = self.up_down_distances.get(current_router,target_router).unwrap_or_else(||panic!("Missing up/down path from {} to {}",current_router,target_router));
		let down_distance = self.down_distances.get(current_router,target_router);
		let num_ports=topology.ports(current_router);
		let mut r=Vec::with_capacity(num_ports*num_virtual_channels);
		for i in 0..num_ports
		{
			//println!("{} -> {:?}",i,topology.neighbour(current_router,i));
			if let (Location::RouterPort{router_index,router_port:_},_link_class)=topology.neighbour(current_router,i)
			{
				let mut label = 0i32;
				let mut new_hops = 0usize;
				let good = if let &Some(down_distance) = down_distance {
					//We can already go down
					let mut good = if let &Some(new_down) = self.down_distances.get(router_index,target_router) {
						label = self.label_down;
						new_hops = new_down.into();
						new_down < down_distance
					} else {
						false
					};
					//or there is some shortcut between branches
					if !good && self.branch_crossings_downwards && self.distance_to_root[router_index]==self.distance_to_root[current_router] {
						if let &Some(new_up_down) = self.up_down_distances.get(router_index,target_router)
						{
							if new_up_down < down_distance
							{
								good = true;
								new_hops = new_up_down.into();
								let delta = (down_distance-1-new_up_down) as usize;
								if let Some(&x) = self.label_horizontal_vec.get(delta) {
									label = x;
								} else {
									label = self.label_horizontal_otherwise;
								}
							}
						}
					}
					good
				} else {
					if let &Some(new_up_down) = self.up_down_distances.get(router_index,target_router)
					{
						//If brach_crossings is false then force to go upwards.
						//new_up_down < up_down_distance && if self.branch_crossings_upwards {
						//	// When branch crossing is allowed we allow horizontal links, but never down-links.
						//	// Allowing down-links can mean deadlock.
						//	self.distance_to_root[router_index]<=self.distance_to_root[current_router]
						//} else {
						//	// If not allowing branch corssing then it must be an up-link.
						//	self.distance_to_root[router_index]<self.distance_to_root[current_router]
						//}
						if new_up_down < up_down_distance {
							label = self.label_up;
							new_hops = new_up_down.into();
							let mut good = self.distance_to_root[router_index]<self.distance_to_root[current_router];
							if !good && self.branch_crossings_upwards && self.distance_to_root[router_index]==self.distance_to_root[current_router] {
								good = true;
								let delta = (up_down_distance-1-new_up_down) as usize;
								if let Some(&x) = self.label_horizontal_vec.get(delta) {
									label = x;
								} else {
									label = self.label_horizontal_otherwise;
								}
							}
							good
						} else { false }
					} else {
						false
					}
				};
				if good{
					r.extend((0..num_virtual_channels).map(|vc|{
						let mut cand = CandidateEgress::new(i,vc);
						cand.label = label;
						cand.estimated_remaining_hops = Some(1+new_hops);
						cand
					}));
				}
			}
		}
		//println!("candidates={:?} current_router={current_router} target_router={target_router} up_down_distance={up_down_distance} down_distance={down_distance:?}",r.iter().map(|x|x.port).collect::<Vec<_>>());
		//println!("From router {} to router {} distance={} cand={}",current_router,target_router,distance,r.len());
		Ok(RoutingNextCandidates{candidates:r,idempotent:true})
	}
	fn initialize(&mut self, topology:&dyn Topology, _rng: &mut StdRng)
	{
		let n = topology.num_routers();
		if let Some(root) = self.root
		{
			self.up_down_distances = Matrix::constant(None,n,n);
			self.down_distances = Matrix::constant(None,n,n);
			//First perform a single BFS at root.
			let mut distance_to_root=vec![None;n];
			distance_to_root[root]=Some(0);
			//A BFS from the root.
			let mut downwards = Vec::with_capacity(n);
			let mut read_index = 0;
			downwards.push(root);
			//for current in 0..n
			while read_index < downwards.len()
			{
				let current = downwards[read_index];
				read_index+=1;
				if let Some(current_distance) = distance_to_root[current]
				{
					let alternate_distance = current_distance + 1;
					for NeighbourRouterIteratorItem{neighbour_router:neighbour,..} in topology.neighbour_router_iter(current)
					{
						if distance_to_root[neighbour].is_none()
						{
							distance_to_root[neighbour]=Some(alternate_distance);
							downwards.push(neighbour);
						}
					}
				}
			}
			self.distance_to_root = distance_to_root.into_iter().map(|d|d.unwrap()).collect();
			//dbg!(&distance_to_root);
			//Second fill assuming going through root
			//dbg!(root,"fill");
			for origin in 0..n
			{
				let origin_to_root = self.distance_to_root[origin];
				for target in 0..n
				{
					let target_to_root = self.distance_to_root[target];
					*self.up_down_distances.get_mut(origin,target) = Some(origin_to_root+target_to_root);
				}
				*self.down_distances.get_mut(root,origin) = Some(origin_to_root);
			}
			//Update the distances considering not reaching the root.
			for origin in 0..n
			{
				*self.up_down_distances.get_mut(origin,origin) = Some(0);
				*self.down_distances.get_mut(origin,origin) = Some(0);
			}
			//dbg!(root,"segments");
			//As invariant: fully computed the higher part (closer to the root).
			for (low_index,&low) in downwards.iter().enumerate()
			{
				for &high in downwards[0..low_index].iter()
				{
					for NeighbourRouterIteratorItem{neighbour_router:neighbour,..} in topology.neighbour_router_iter(low)
					{
						if self.distance_to_root[neighbour]+1==self.distance_to_root[low]
						{
							//neighbour is upwards
							let neighbour_up_down = self.up_down_distances.get(neighbour,high).unwrap();
							let origin_up_down = self.up_down_distances.get(low,high).unwrap();
							if neighbour_up_down+1 < origin_up_down
							{
								*self.up_down_distances.get_mut(low,high) = Some(neighbour_up_down+1);
								*self.up_down_distances.get_mut(high,low) = Some(neighbour_up_down+1);
							}
							if let Some(neighbour_down) = self.down_distances.get(high,neighbour)
							{
								if self.down_distances.get(high,low).map(|origin_down|neighbour_down+1<origin_down).unwrap_or(true)
								{
									//println!("high={high} neighbour={neighbour} low={low} distance={}",neighbour_down+1);
									*self.down_distances.get_mut(high,low) = Some(neighbour_down+1);
								}
							}
						}
					}
				}
			}
			//dbg!(&self.up_down_distances);
			//for origin in 0..n
			//{
			//	//Start towards root annotating those that require only upwards.
			//	//let _origin_to_root) = distance_to_root[origin];
			//	let mut upwards=Vec::with_capacity(n);
			//	upwards.push((origin,0));
			//	let mut read_index = 0;
			//	while read_index < upwards.len()
			//	{
			//		let (current,distance) = upwards[read_index];
			//		let current_to_root = distance_to_root[current];
			//		read_index+=1;
			//		*self.up_down_distances.get_mut(origin,current)=Some((distance,0));
			//		*self.up_down_distances.get_mut(current,origin)=Some((0,distance));
			//		for NeighbourRouterIteratorItem{neighbour_router:neighbour,..} in topology.neighbour_router_iter(current)
			//		{
			//			let neighbour_to_root = distance_to_root[neighbour];
			//			if neighbour_to_root +1 == current_to_root
			//			{
			//				upwards.push((neighbour,distance+1));
			//			}
			//		}
			//	}
			//}
			//dbg!(root,"finished table");
		}
		if n!=self.up_down_distances.get_columns()
		{
			panic!("ExplicitUpDown has not being properly initialized");
		}
	}
	//fn initialize_routing_info(&self, routing_info:&RefCell<RoutingInfo>, _topology:&dyn Topology, _current_router:usize, _target_server:usize, _rng: &mut StdRng)
	//{
	//	routing_info.borrow_mut().selections=Some(Vec::new());
	//}
	//fn update_routing_info(&self, routing_info:&RefCell<RoutingInfo>, topology:&dyn Topology, current_router:usize, _current_port:usize, _target_server:usize, _rng: &mut StdRng)
	//{
	//	let mut bri = routing_info.borrow_mut();
	//	let v = bri.selections.as_mut().unwrap();
	//	let root = *self.root.as_ref().unwrap();
	//	let distance = topology.distance(root,current_router);
	//	v.push(distance as i32);
	//	println!("distances={v:?} current_router={current_router}");
	//}
}

impl ExplicitUpDown
{
	pub fn new(arg: RoutingBuilderArgument) -> ExplicitUpDown
	{
		let mut root = None;
		let mut branch_crossings_downwards = false;
		let mut branch_crossings_upwards = false;
		let mut label_down = 0i32;
		let mut label_up = 0i32;
		let mut label_horizontal_vec = vec![];
		let mut label_horizontal_otherwise = 0i32;
		match_object_panic!(arg.cv,"UpDownStar",value,
			"root" => root=Some(value.as_f64().expect("bad value for root") as usize),
			"branch_crossings" => {
				branch_crossings_upwards = value.as_bool().expect("bad value for branch_crossings");
				branch_crossings_downwards = branch_crossings_upwards;
			},
			"branch_crossings_upwards" => branch_crossings_upwards=value.as_bool().expect("bad value for branch_crossings_upwards"),
			"branch_crossings_downwards" => branch_crossings_downwards=value.as_bool().expect("bad value for branch_crossings_downwards"),
			"label_up" | "label_upwards" => label_up = value.as_i32().expect("bad value for label_up"),
			"label_down" | "label_downwards" => label_down = value.as_i32().expect("bad value for label_down"),
			"label_horizontal_vec" => label_horizontal_vec = value.as_array().expect("bad value for label_horizontal_vec").iter().map(|x|{
				x.as_i32().expect("bad value for label_horizontal_vec entry")
			}).collect(),
			"label_horizontal_otherwise" => label_horizontal_otherwise = value.as_i32().expect("bad value for label_horizontal_otherwise"),
		);
		ExplicitUpDown{
			root,
			up_down_distances: Matrix::constant(None,0,0),
			down_distances: Matrix::constant(None,0,0),
			distance_to_root: Vec::new(),
			branch_crossings_downwards,
			branch_crossings_upwards,
			label_down,
			label_up,
			label_horizontal_vec,
			label_horizontal_otherwise,
		}
	}
}
///Routing for indirect networks which follows up-down routes adaptively.
#[derive(Debug)]
pub struct UpDownDerouting
{
	///Maximum number of non-shortest (deroutes) hops to make.
	allowed_updowns: usize,
	/// (Optional): VC to take in each UpDown stage. By default one different VC per UpDown path.
	virtual_channels: Vec<Vec<usize>>,
	/// Stages in the multistage, by the default 1.
	stages: usize,
	routing_table: Rc<RefCell<RoutingTable>>,
}
impl Routing for UpDownDerouting
{
	fn next(&self, routing_info: &RoutingInfo, topology: &dyn Topology, current_router: usize, target_router: usize, target_server: Option<usize>, num_virtual_channels: usize, _rng: &mut StdRng) -> Result<RoutingNextCandidates, Error> {
		let num_ports=topology.ports(current_router);
		let mut r=Vec::with_capacity(num_ports*num_virtual_channels);

		//let avaliable_updown_deroutes = routing_info.selections.as_ref().unwrap()[0] as usize; // asigna el numero de saltos permitidos
		//let vc_index= self.allowed_updowns - avaliable_updown_deroutes;
		let distance = topology.distance(current_router, target_router);
		match distance {
			0 => {// CASO 1: el router actual es el router destino
				let target_server = target_server.expect("target server was not given.");
				for i in 0..topology.ports(current_router)
				{
					if let (Location::ServerPort(server),_link_class)=topology.neighbour(current_router,i)
					{
						if server==target_server
						{
							return Ok(RoutingNextCandidates{candidates:(0..num_virtual_channels).map(|vc|CandidateEgress::new(i,vc)).collect(),idempotent:true});
						}
					}
				}
				unreachable!();
			},
			1 => { // CASO 2: el router actual esta a un salto del destino
				for i in 0..num_ports
				{
					if let (Location::RouterPort{router_index,router_port:_},_link_class)=topology.neighbour(current_router,i)
					{
						// si la distancia entre el vecino y el destino es 0, entonces es el router destino
						if topology.distance(router_index, target_router) == 0
						{
							//r.extend((0..num_virtual_channels).map(|vc|CandidateEgress::new(i,vc)));
							r.extend(self.virtual_channels[0].iter().map(|&vc|CandidateEgress::new(i,vc)));
						}
					}
				}
			},
			2 => { // CASO 3: el router tiene un camino up/down hacia el destino
				let next_port = self.routing_table.borrow().next_router_1(current_router, target_router);
				r.extend(self.virtual_channels[0].iter().map(|&vc|CandidateEgress::new(next_port,vc)));
			},
			3 => { // CASO 4: el router actual esta a 3 saltos del destino
				let next_port = self.routing_table.borrow().next_router_2(current_router, target_router, topology);
				r.extend(self.virtual_channels[1].iter().map(|&vc|CandidateEgress::new(next_port,vc)));
			},
			4 => { // CASO 5: el router actual esta a 4 saltos o dos up/down del destino
				let next_port = self.routing_table.borrow().next_router_3(current_router, target_router, topology);
				r.extend(self.virtual_channels[1].iter().map(|&vc|CandidateEgress::new(next_port,vc)));	
				println!("!!!! distancia 4");
			},
			_ => panic!("rutas largas"),
		}
		Ok(RoutingNextCandidates{candidates:r,idempotent:true})
	}
	fn initialize_routing_info(&self, routing_info:&RefCell<RoutingInfo>, _topology:&dyn Topology, current_router:usize, _target_touter:usize, _target_server:Option<usize>, _rng: &mut StdRng)
	{
		routing_info.borrow_mut().selections=Some(vec![self.allowed_updowns as i32]);
		routing_info.borrow_mut().visited_routers=Some(vec![current_router]);
		routing_info.borrow_mut().auxiliar= RefCell::new(Some(Box::new(vec![0usize;self.stages])));
	}
	fn update_routing_info(&self, routing_info:&RefCell<RoutingInfo>, topology:&dyn Topology, current_router:usize, current_port:usize, target_router:usize, _target_server:Option<usize>,_rng: &mut StdRng)
	{
		if let (Location::RouterPort{router_index: _previous_router,router_port:_},link_class)=topology.neighbour(current_router,current_port)
		{
			let mut bri=routing_info.borrow_mut();
			let aux = bri.auxiliar.borrow_mut().take().unwrap();
			let mut saltos =  aux.downcast_ref::<Vec<usize>>().unwrap().clone();
			if saltos[link_class] != 0
			{
				saltos[link_class] = 0usize;
				if link_class == 0  && current_router != target_router// now we are in last stage
				{
					match bri.selections
					{
						Some(ref mut v) =>
							{
								let available_updown_deroutes=v[0];
								if available_updown_deroutes==0
								{
									panic!("Bad deroute :(");
								}
								v[0]= available_updown_deroutes-1;
							}
						None => panic!("selections not initialized"),
					};
				}
			}
			else
			{
				saltos[link_class] = 1usize;
			}

			bri.auxiliar.replace(Some(Box::new(saltos)));

			match bri.visited_routers
			{
				Some(ref mut v) =>
				{
					v.push(current_router);
				}
				None => panic!("visited_routers not initialized"),
			};

		}
	}
	fn initialize(&mut self, topology: &dyn Topology, _rng: &mut StdRng) {
		self.routing_table.borrow_mut().build_distance_2_paths(topology);
		self.routing_table.borrow_mut().build_leaf_routers(topology);
		// self.routing_table.borrow_mut().build_distance_2_rutas(topology);
		// self.routing_table.borrow_mut().print_paths();
		// self.routing_table.borrow_mut().print_rutas();
	}
	fn performed_request(&self, _requested:&CandidateEgress, _routing_info:&RefCell<RoutingInfo>, _topology:&dyn Topology, _current_router:usize, _target_router:usize, _target_server:Option<usize>, _num_virtual_channels:usize, _rng:&mut StdRng)
	{
	}
	fn statistics(&self, _cycle:Time) -> Option<ConfigurationValue>
	{
		return None;
	}
	fn reset_statistics(&mut self, _next_cycle:Time)
	{
	}
}
impl UpDownDerouting {
    pub fn new(arg: RoutingBuilderArgument) -> UpDownDerouting {
        let mut allowed_updowns = None;
        let mut stages = 1usize;
        let mut virtual_channels = None;
		let _routing_table = RoutingTable::new();

        if let &ConfigurationValue::Object(ref cv_name, ref cv_pairs) = arg.cv {
            if cv_name != "UpDownDerouting" {
                panic!("A UpDownDerouting must be created from a `UpDownDerouting` object not `{}`", cv_name);
            }
            for &(ref name, ref value) in cv_pairs {
                match AsRef::<str>::as_ref(&name) {
                    "allowed_updowns" => match value {
                        &ConfigurationValue::Number(f) => allowed_updowns = Some(f as usize),
                        _ => panic!("bad value for allowed_deroutes"),
                    },
                    "stages" => match value {
                        &ConfigurationValue::Number(f) => stages = f as usize,
                        _ => (),
                    },
                    "virtual_channels" => match value {
                        ConfigurationValue::Array(f) => virtual_channels = Some(f.into_iter().map(|a| a.as_array().unwrap().into_iter().map(|b| b.as_usize().unwrap()).collect()).collect()),
                        _ => (),
                    },
                    "legend_name" => (),
                    _ => panic!("Nothing to do with field {} in UpDownDerouting", name),
                }
            }
        } else {
            panic!("Trying to create a UpDownDerouting from a non-Object");
        }

        let allowed_updowns = allowed_updowns.expect("There were no allowed_deroutes");

        let virtual_channels = match virtual_channels {
            Some(v) => v,
            None => {
				let a= vec![0;allowed_updowns];
				a.iter().enumerate().map(|(i,_vc)|vec![i]).collect::<Vec<Vec<usize>>>()
			}
        };

        UpDownDerouting {
            allowed_updowns,
            virtual_channels,
            stages,
			routing_table: Rc::new(RefCell::new(RoutingTable::new())),
        }
    }
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Plugs;
	use rand::SeedableRng;
	use crate::topology::cartesian::Hamming;
	#[test]
	fn up_down_star()
	{
		let plugs = Plugs::default();
		let uds_cv = ConfigurationValue::Object("UpDownStar".to_string(),vec![("root".to_string(),ConfigurationValue::Number(5.0))]);
		let uds_arg = RoutingBuilderArgument{cv:&uds_cv,plugs:&plugs};
		let mut uds = ExplicitUpDown::new(uds_arg);
		let mut rng=StdRng::seed_from_u64(10u64);
		let hamming_cv = ConfigurationValue::Object("Hamming".to_string(),vec![("sides".to_string(),ConfigurationValue::Array(vec![
			ConfigurationValue::Number(8.0),
			ConfigurationValue::Number(8.0),
		])),("servers_per_router".to_string(),ConfigurationValue::Number(8.0))]);
		let topology = Hamming::new(&hamming_cv);
		uds.initialize(&topology,&mut rng);
		let n = topology.num_routers();
		for origin in 0..n
		{
			for destination in 0..n
			{
				let origin_ud = uds.up_down_distances.get(origin,destination).expect("missing an up/down distance");
				let is_down = uds.down_distances.get(origin,destination).is_some();
				// Count neighbours that reduce the up/down distance.
				let mut count_improvers = 0;
				for NeighbourRouterIteratorItem{neighbour_router:neighbour,..} in topology.neighbour_router_iter(origin)
				{
					let neighbour_ud = uds.up_down_distances.get(neighbour,destination).expect("missing an up/down distance");
					if neighbour_ud < origin_ud && (is_down || uds.distance_to_root[origin]==uds.distance_to_root[neighbour]+1) {
						count_improvers +=1;
					}
				}
				assert!(origin==destination || count_improvers>=1,"origin={} destination={} ud={}",origin,destination,origin_ud);
			}
		}
	}
}

