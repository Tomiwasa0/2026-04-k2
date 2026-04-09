#![cfg(test)]
use super::*;
use soroban_sdk::{testutils::Address as _, Address, Env};

#[test]
fn test_initialize() {
    let env = Env::default();
    let contract_id = env.register(SoroswapSwapAdapter, ());
    let client = SoroswapSwapAdapterClient::new(&env, &contract_id);
    
    let admin = Address::generate(&env);
    let router = Address::generate(&env);
    let factory = Address::generate(&env);
    
    env.mock_all_auths();
    
    client.initialize(&admin, &router, &Some(factory.clone()));
    
    assert_eq!(client.get_router(), router);
}

#[test]
fn test_initialize_without_factory() {
    let env = Env::default();
    let contract_id = env.register(SoroswapSwapAdapter, ());
    let client = SoroswapSwapAdapterClient::new(&env, &contract_id);
    
    let admin = Address::generate(&env);
    let router = Address::generate(&env);
    
    env.mock_all_auths();
    
    client.initialize(&admin, &router, &None);
    
    assert_eq!(client.get_router(), router);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_double_initialize() {
    let env = Env::default();
    let contract_id = env.register(SoroswapSwapAdapter, ());
    let client = SoroswapSwapAdapterClient::new(&env, &contract_id);
    
    let admin = Address::generate(&env);
    let router = Address::generate(&env);
    
    env.mock_all_auths();
    
    client.initialize(&admin, &router, &None);
    client.initialize(&admin, &router, &None); // Should fail with AlreadyInitialized (#2)
}

#[test]
fn test_set_router() {
    let env = Env::default();
    let contract_id = env.register(SoroswapSwapAdapter, ());
    let client = SoroswapSwapAdapterClient::new(&env, &contract_id);
    
    let admin = Address::generate(&env);
    let router = Address::generate(&env);
    let new_router = Address::generate(&env);
    
    env.mock_all_auths();
    
    client.initialize(&admin, &router, &None);
    client.set_router(&admin, &new_router);
    
    assert_eq!(client.get_router(), new_router);
}
