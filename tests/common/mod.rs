use caminos_lib::*;
use config_parser::ConfigurationValue;

/*
    Auxiliary functions to create the configuration file for the tests. Each function has a struct as argument wich contains the needed parameters
*/

/// Encapsulates the parameters needed to create a Virtual Channel Policy
pub struct VirtualChannelPoliciesBuilder{

    pub policies: Vec<ConfigurationValue>,
}

/// Creates a Configuration Value with the policy for the virtual channels
pub fn create_vcp( arg: VirtualChannelPoliciesBuilder) -> ConfigurationValue
{
    ConfigurationValue::Array(arg.policies)
}

/// Encapsulates the parameters needed to create a Input_output router
pub struct InputOutputRouterBuilder {
    pub virtual_channels: usize,
    pub vcp: ConfigurationValue,
    pub crossbar_delay: usize,
    pub crossbar_frequency_divisor: usize,
    pub allocator: ConfigurationValue,
    pub buffer_size: usize,
    pub bubble: ConfigurationValue,
    pub flit_size: usize,
    pub allow_request_busy_port: ConfigurationValue,
    pub intransit_priority: ConfigurationValue,
    pub output_buffer_size: usize,
    pub neglect_busy_outport: ConfigurationValue,
}

/// Creates a Configuration Value with the parameters for the Input_output router
pub fn create_input_output_router(arg: InputOutputRouterBuilder)-> ConfigurationValue
{
        ConfigurationValue::Object("InputOutput".to_string(), vec![
            ("virtual_channels".to_string(),ConfigurationValue::Number( arg.virtual_channels as f64 )),
            ("virtual_channel_policies".to_string(), arg.vcp),
            ("allocator".to_string(), arg.allocator),
            ("crossbar_delay".to_string(), ConfigurationValue::Number(arg.crossbar_delay as f64) ),
            ("crossbar_frequency_divisor".to_string(), ConfigurationValue::Number(arg.crossbar_frequency_divisor as f64)),
            ("buffer_size".to_string(), ConfigurationValue::Number(arg.buffer_size as f64)),
            ("bubble".to_string(), arg.bubble),
            ("flit_size".to_string(), ConfigurationValue::Number(arg.flit_size as f64) ),
            ("allow_request_busy_port".to_string(), arg.allow_request_busy_port),
            ("intransit_priority".to_string(), arg.intransit_priority),
            ("output_buffer_size".to_string(), ConfigurationValue::Number(arg.output_buffer_size as f64)),
            ("neglect_busy_output".to_string(), arg.neglect_busy_outport)
    ])
}
/// Encapsulates the parameters needed to create a Basic router
pub struct BasicRouterBuilder {
    pub virtual_channels: usize,
    pub vcp: ConfigurationValue,
    pub buffer_size: usize,
    pub bubble: ConfigurationValue,
    pub flit_size: usize,
    pub allow_request_busy_port: ConfigurationValue,
    pub intransit_priority: ConfigurationValue,
    pub output_buffer_size: usize,
    pub neglect_busy_outport: ConfigurationValue,
    pub output_prioritize_lowest_label: ConfigurationValue,
}
/// Creates a Configuration Value with the parameters for the Basic router
pub fn create_basic_router( arg: BasicRouterBuilder)-> ConfigurationValue
{
    ConfigurationValue::Object("Basic".to_string(), vec![
        ("virtual_channels".to_string(),ConfigurationValue::Number(arg.virtual_channels as f64)),
        ("virtual_channel_policies".to_string(), arg.vcp),
        ("buffer_size".to_string(), ConfigurationValue::Number(arg.buffer_size as f64)),
        ("bubble".to_string(), arg.bubble),
        ("flit_size".to_string(), ConfigurationValue::Number(arg.flit_size as f64) ),
        ("allow_request_busy_port".to_string(), arg.allow_request_busy_port),
        ("intransit_priority".to_string(), arg.intransit_priority),
        ("output_buffer_size".to_string(), ConfigurationValue::Number(arg.output_buffer_size as f64)),
        ("neglect_busy_output".to_string(), arg.neglect_busy_outport),
        ("output_prioritize_lowest_label".to_string(), arg.output_prioritize_lowest_label)
    ])

}

/// Encapsulates the parameters needed to create a HyperX topology
pub struct HammingBuilder
{
    pub sides: Vec<ConfigurationValue>,
    pub servers_per_router: usize,
}
/// Creates a Configuration Value with the parameters for the HyperX topology
pub fn create_hamming_topology(arg: HammingBuilder) -> ConfigurationValue //Box<dyn Topology>
{
    ConfigurationValue::Object("Hamming".to_string(),
                                        vec![("sides".to_string(),ConfigurationValue::Array(arg.sides)),
                                             ("servers_per_router".to_string(),ConfigurationValue::Number(arg.servers_per_router as f64))])
}

/// Encapsulates the parameters needed to create a Cartesian shift pattern (x, y) -> (x + shift_x, y + shift_y)
/// DEPRECATED - use cartesian_pattern_builder
pub struct ShiftPatternBuilder
{
    pub sides: Vec<ConfigurationValue>,
    pub shift: Vec<ConfigurationValue>,
}
/// Creates a Configuration Value with the parameters for the Cartesian shift pattern
pub fn create_shift_pattern(arg: ShiftPatternBuilder) -> ConfigurationValue
{
    ConfigurationValue::Object("CartesianTransform".to_string(), vec![
        ("sides".to_string(),ConfigurationValue::Array(arg.sides)),
        ("shift".to_string(),ConfigurationValue::Array(arg.shift))])
}

/// Encapsulates the parameters needed to create a Cartesian pattern
pub struct CartesianPatternBuilder
{
    pub sides: Vec<ConfigurationValue>,
    pub shift: Option<Vec<ConfigurationValue>>,
    pub permute: Option<Vec<ConfigurationValue>>,
    pub complement: Option<Vec<ConfigurationValue>>,
    pub project: Option<Vec<ConfigurationValue>>,
    pub random: Option<Vec<ConfigurationValue>>,
    pub patterns: Option<Vec<ConfigurationValue>>,
}
/// Creates a Configuration Value with the parameters for the Cartesian pattern
pub fn create_cartesian_pattern(arg: CartesianPatternBuilder) -> ConfigurationValue
{
    let mut vec = vec![("sides".to_string(),ConfigurationValue::Array(arg.sides))];
    if let Some(shift) = arg.shift {
        vec.push(("shift".to_string(),ConfigurationValue::Array(shift)));
    }
    if let Some(permute) = arg.permute {
        vec.push(("permute".to_string(),ConfigurationValue::Array(permute)));
    }
    if let Some(complement) = arg.complement {
        vec.push(("complement".to_string(),ConfigurationValue::Array(complement)));
    }
    if let Some(project) = arg.project {
        vec.push(("project".to_string(),ConfigurationValue::Array(project)));
    }
    if let Some(random) = arg.random {
        vec.push(("random".to_string(),ConfigurationValue::Array(random)));
    }
    if let Some(patterns) = arg.patterns {
        vec.push(("patterns".to_string(),ConfigurationValue::Array(patterns)));
    }
    ConfigurationValue::Object("CartesianTransform".to_string(), vec)
}


/// Encapsulates the parameters needed to create a Burst traffic pattern.
pub struct BurstTrafficBuilder
{
    pub pattern: ConfigurationValue,
    pub servers: usize,
    pub messages_per_server: usize,
    pub message_size: usize,
}
/// Creates a Configuration Value with the parameters for the Burst traffic pattern
pub fn create_burst_traffic(arg: BurstTrafficBuilder) -> ConfigurationValue
{
    ConfigurationValue::Object("Burst".to_string(), vec![("pattern".to_string(), arg.pattern ),
                                                         ("servers".to_string(), ConfigurationValue::Number(arg.servers as f64)),
                                                         ("messages_per_server".to_string(), ConfigurationValue::Number(arg.messages_per_server as f64)),
                                                         ("message_size".to_string(), ConfigurationValue::Number(arg.message_size as f64))])
}

/// Creates a Configuration Value for Shortest routing
pub fn create_shortest_routing() -> ConfigurationValue
{
    ConfigurationValue::Object("Shortest".to_string(), vec![])
}

///Creates a Configuration Value for DOR
pub fn create_dor_routing(order: Vec<usize>) -> ConfigurationValue
{
    ConfigurationValue::Object("DOR".to_string(), vec![("order".to_string(), ConfigurationValue::Array( order.into_iter().map(|a| ConfigurationValue::Number(a as f64)).collect() ))])
}

///Creates a Configuration Value for Omnidimensional routing
pub fn create_omnidimensional_routing(allowed_deroutes: ConfigurationValue, include_labels: ConfigurationValue) -> ConfigurationValue
{
    ConfigurationValue::Object("OmniDimensionalDeroute".to_string(), vec![
        ("allowed_deroutes".to_string(), allowed_deroutes),
        ("include_labels".to_string(), include_labels)]
    )
}

/// Creates a Configuration Value for link classes
pub fn create_link_classes() -> ConfigurationValue
{
   ConfigurationValue::Array(vec![
       ConfigurationValue::Object("LinkClass".to_string(), vec![("delay".to_string(), ConfigurationValue::Number(1.0))]),
       ConfigurationValue::Object("LinkClass".to_string(), vec![("delay".to_string(), ConfigurationValue::Number(1.0))]),
       ConfigurationValue::Object("LinkClass".to_string(), vec![("delay".to_string(), ConfigurationValue::Number(1.0))]),
       ConfigurationValue::Object("LinkClass".to_string(), vec![("delay".to_string(), ConfigurationValue::Number(1.0))]),
       ConfigurationValue::Object("LinkClass".to_string(), vec![("delay".to_string(), ConfigurationValue::Number(1.0))]),
   ])
}

/// Encapsulates the parameters needed to init a simulation
pub struct SimulationBuilder
{
    pub random_seed: usize,
    pub warmup: usize,
    pub measured: usize,
    pub topology: ConfigurationValue,
    pub traffic: ConfigurationValue,
    pub router: ConfigurationValue,
    pub maximum_packet_size: usize,
    pub general_frequency_divisor: usize,
    pub routing: ConfigurationValue,
    pub link_classes: ConfigurationValue

}

/// Creates a Configuration Value with all the fields for a simulation
pub fn create_simulation(arg: SimulationBuilder) -> ConfigurationValue
{

    ConfigurationValue::Object("Configuration".to_string(), vec![
        ("random_seed".to_string(), ConfigurationValue::Number(arg.random_seed as f64)),
        ("warmup".to_string(), ConfigurationValue::Number( arg.warmup as f64)),
        ("measured".to_string(), ConfigurationValue::Number(arg.measured as f64)),
        ("topology".to_string(), arg.topology),
        ("traffic".to_string(), arg.traffic),
        ("router".to_string(), arg.router),
        ("maximum_packet_size".to_string(), ConfigurationValue::Number(arg.maximum_packet_size as f64)),
        ("general_frequency_divisor".to_string(), ConfigurationValue::Number(arg.general_frequency_divisor as f64)),
        ("routing".to_string(), arg.routing),
        ("link_classes".to_string(), arg.link_classes),
    ])

}
